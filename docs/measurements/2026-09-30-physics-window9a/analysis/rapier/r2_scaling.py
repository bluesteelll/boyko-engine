"""r2: REPORTED, NOT CLAIMED (window9a_rows.md): T(1)/T(W) per engine; Rapier vs Jolt; ns per row-iteration;
the V2 arithmetic (NOT a measurement); pass-to-pass drift inside each block. Cells = pooled medians (lib_rp.cell)."""
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_rp as L  # noqa: E402

o = L.Out()
recs = L.all_records()
_, U_R, _ = L.select('C4-RAPIER', recs)
_, U_A, _ = L.select('C4-AB', recs, {'C4-JT', 'C4-JD', 'C4-jolt56'})
WS = (1, 2, 4, 8, 16)
OURS_ROWS = {(0, 100): 588764.88, (100, 500): 595935.18}
WN = {(100, 500): '[100,500)', (0, 100): '[0,100)', (0, 500): '[0,500)'}


def med(used, row, b, W, win):
    v = L.by_pass(used, row, b, W, win)
    c = L.cell([x for k in (0, 1, 2) for x in v[k]])
    return c['median'] / 1e6, c['K']


ENG = [('ours (J-T 1/8/16, J-D 2/4)', lambda W, w: med(U_A, 'C4-JT' if W in (1, 8, 16) else 'C4-JD', 'tip', W, w)),
       ('ours J-D all W (post hoc)', lambda W, w: med(U_A, 'C4-JD', 'tip', W, w)),
       ('Rapier RP-D rs8', lambda W, w: med(U_R, 'RP-D', 'rs8', W, w)),
       ('Rapier RP-D rs4', lambda W, w: med(U_R, 'RP-D', 'rs4', W, w)),
       ('Rapier RP-M rs8', lambda W, w: med(U_R, 'RP-M', 'rs8', W, w)),
       ('Rapier RP-M rs4', lambda W, w: med(U_R, 'RP-M', 'rs4', W, w)),
       ('Jolt 5.6', lambda W, w: med(U_A, 'C4-jolt56', 'j56', W, w))]

for win in ((100, 500), (0, 500), (0, 100)):
    o(f'=== T(W) ms (pooled median, n) and T(1)/T(W), window {WN[win]} ===')
    o(f'  {"engine":30s} ' + ' '.join(f'{"W" + str(W):>16s}' for W in WS) + '   best W   T16/T8')
    for name, f in ENG:
        t = {W: f(W, win) for W in WS}
        cells = ' '.join(f'{t[W][0]:7.4f}(n{t[W][1]}) {t[1][0] / t[W][0]:4.2f}x' for W in WS)
        best = min(WS, key=lambda W: t[W][0])
        o(f'  {name:30s} {cells}   W{best:<3d}   {t[16][0] / t[8][0]:.3f}')
    o()

o('=== ns per row-iteration (pooled median wall / rows of the window), [100,500) and [0,100) ===')
for win in ((100, 500), (0, 100)):
    o(f'--- {WN[win]} (rows: ours {OURS_ROWS[win]:,.0f}; RP-D {L.RAPIER_ROWS["RP-D"][win]:,}; RP-M {L.RAPIER_ROWS["RP-M"][win]:,}) ---')
    for W in WS:
        ours = ENG[0][1](W, win)[0] * 1e6 / OURS_ROWS[win]
        parts = [f'ours {ours:.3f}']
        for cfg in ('RP-D', 'RP-M'):
            for a in ('rs8', 'rs4'):
                v = med(U_R, cfg, a, W, win)[0] * 1e6 / L.RAPIER_ROWS[cfg][win]
                parts.append(f'{cfg} {a} {v:.3f}')
        o(f'  W{W:<2d} ' + '  '.join(parts))
o()

o('=== Rapier vs Jolt 5.6 (REPORTED, NOT CLAIMED; cross-block): Rapier median / Jolt median ===')
for win in ((100, 500), (0, 500)):
    o(f'--- {WN[win]} ---')
    for W in WS:
        j = med(U_A, 'C4-jolt56', 'j56', W, win)[0]
        parts = [f'{c} {a} {med(U_R, c, a, W, win)[0] / j:.3f}' for c in ('RP-D', 'RP-M') for a in ('rs8', 'rs4')]
        ours = ENG[0][1](W, win)[0]
        o(f'  W{W:<2d} Jolt {j:.4f} ms; ' + '  '.join(parts) + f'   | ours/Jolt {ours / j:.3f}')
o()

o('=== V2 ARITHMETIC (NOT A MEASUREMENT): ours after V2 if our ns/row stayed at this window value ===')
o('  rows_V2 per audit.md section 3: F0 S20 1.676x ours = 998,787; with K3 (+~7 %) ~1,068,700. Rapier RP-M 1,071,107.')
for W in WS:
    ours = ENG[0][1](W, (100, 500))[0]
    lo, hi = ours * 998787 / OURS_ROWS[(100, 500)], ours * 1068700 / OURS_ROWS[(100, 500)]
    best_m = min(med(U_R, 'RP-M', a, W, (100, 500))[0] for a in ('rs8', 'rs4'))
    best_d = min(med(U_R, 'RP-D', a, W, (100, 500))[0] for a in ('rs8', 'rs4'))
    o(f'  W{W:<2d} ours now {ours:.4f} ms -> V2 {lo:.4f}-{hi:.4f} ms; best RP-M {best_m:.4f} (ours_V2/RP-M {lo / best_m:.3f}-{hi / best_m:.3f}); '
      f'best RP-D {best_d:.4f} (ours_V2/RP-D {lo / best_d:.3f}-{hi / best_d:.3f})')
o()

o('=== pass-to-pass drift inside each block: per-pass median / pooled median, [100,500) ===')
for label_, used, cells in (('C4-RAPIER', U_R, [(c, a) for c in ('RP-D', 'RP-M') for a in ('rs8', 'rs4')]),
                            ('C4-AB', U_A, [('C4-JT', 'tip'), ('C4-JD', 'tip'), ('C4-jolt56', 'j56')])):
    per = {0: [], 1: [], 2: []}
    for (row, b) in cells:
        for W in WS:
            v = L.by_pass(used, row, b, W, (100, 500))
            allv = [x for k in (0, 1, 2) for x in v[k]]
            if not allv:
                continue
            m = statistics.median(allv)
            for k in (0, 1, 2):
                if v[k]:
                    per[k].append((statistics.median(v[k]) / m, f'{row}#{b}@W{W}', len(v[k])))
    for k in (0, 1, 2):
        xs = [x for x, _, _ in per[k]]
        worst = max(per[k], key=lambda t: abs(t[0] - 1))
        o(f'  {label_}-p{k}: cells {len(xs)}, median ratio {statistics.median(xs):.4f}, range {min(xs):.4f}-{max(xs):.4f}; '
          f'largest {worst[1]} {worst[0]:.4f} (K{worst[2]})')
o()
o('=== C4-RAPIER p0, W1 group (the last group of the reversed pass): process means [100,500) ms and receipts ===')
for p in sorted([p for p in U_R if p['pass'] == 0 and p['W'] == 1], key=lambda p: p['rec']['start']):
    r = p['rec']
    o(f'  {r["start"][11:19]} {p["row"]}#{p["binary"]} r{p["round"]}: {p["vals"][(100, 500)] / 1e6:.4f}  '
      f'before {r["receipt_before"]["cpu_avg"]:.2f} after {r["receipt_after"]["cpu_avg"]:.2f} witness {r["others_busy_pct"]:.2f}')
o.save('r2_scaling.txt')
o()
o('=== row ratios (the R-WALL-M equal-work band is [0.90, 1.10]) ===')
for win in ((100, 500), (0, 100)):
    rm = L.RAPIER_ROWS['RP-M'][win] / OURS_ROWS[win]
    rd = L.RAPIER_ROWS['RP-D'][win] / OURS_ROWS[win]
    o(f'  {WN[win]}: rows RP-M / ours {rm:.4f} (in band: {0.90 <= rm <= 1.10}); rows RP-D / ours {rd:.4f}')
o('  V2 projection (audit.md s3): 998,787 / 595,935 = %.4f ; 1,068,700 / 595,935 = %.4f ; RP-M / V2 = %.4f-%.4f' % (
    998787 / OURS_ROWS[(100, 500)], 1068700 / OURS_ROWS[(100, 500)], 1071107 / 1068700, 1071107 / 998787))
o.save('r2_scaling.txt')
