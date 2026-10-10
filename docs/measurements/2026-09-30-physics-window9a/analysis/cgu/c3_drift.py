"""C4-CGU POST HOC: the cross-block reading of the tip (cgu16) cells, C4-CGU (run 033740, 2026-10-01 03:40-03:58)
against C4-AB's C4-JD#tip (run 191354, 2026-09-30 19:14-20:43). Same exe (8d6e7d41), same args. Per pass,
receipts, placement, process CPU, and the per-step distribution inside [100,500). Decides nothing."""
import statistics

import cgu_lib as C

L = C.L
o = C.Out()
recs = C.all_records()
_, used, _ = C.select(C.BLOCK, recs, C.RUN_CGU, row=C.ROW)
_, ab_used, _ = C.select('C4-AB', recs, C.RUN_AB, row='C4-JD', binary='tip')
cg_tip = [p for p in used if p['binary'] == C.A_KEY]
cg_b = [p for p in used if p['binary'] == C.B_KEY]
o('# C4-CGU tip vs C4-AB C4-JD#tip (POST HOC, cross-block)')
for W in C.WS:
    va = C.vals(ab_used, 'tip', W, C.PRIMARY, row='C4-JD')
    vb = C.vals(cg_tip, C.A_KEY, W, C.PRIMARY)
    j = L.judge(va, vb, 'GATING-ONLY')
    o(f'W{W}: ' + ' '.join(f'p{k}:{L.fper(c)}' for k, c in j['per'].items()) + f'  pooled {L.yn(j["pooled"])} bars i {100*j["pooled"]["bar_i"]:.2f} s {100*j["pooled"]["bar_s"]:.2f}')


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else float('nan')


o('\nper W, medians over used processes: receipt before / after %, witness %, main_cycle_frac, top_share_of_proc, proc_cpu_s / wall_s, wall_s')
for W in C.WS:
    for name, ps in (('AB  tip ', [p for p in ab_used if p['W'] == W]), ('CGU tip ', [p for p in cg_tip if p['W'] == W]),
                     ('CGU cgu1', [p for p in cg_b if p['W'] == W])):
        r = [p['rec'] for p in ps]
        o(f'  W{W:<2d} {name} n={len(r)}: rb {med([x["receipt_before"]["cpu_avg"] for x in r]):.2f} ra {med([x["receipt_after"]["cpu_avg"] for x in r]):.2f} '
          f'wit {med([x["others_busy_pct"] for x in r]):.2f} mainfrac {med([x["placement"].get("main_cycle_frac") for x in r]):.4f} '
          f'topshare {med([x["placement"].get("top_share_of_proc") for x in r]):.3f} cpu/wall {med([x["proc_cpu_s"]/x["wall_s"] for x in r]):.2f} '
          f'wall {med([x["wall_s"] for x in r]):.3f} s')

o('\nper-step distribution inside [100,500) (ms): median over processes of the step p10 / p50 / p90 and of the window mean')
for W in (1, 8, 16):
    for name, ps in (('AB  tip ', [p for p in ab_used if p['W'] == W]), ('CGU tip ', [p for p in cg_tip if p['W'] == W]),
                     ('CGU cgu1', [p for p in cg_b if p['W'] == W])):
        q10, q50, q90, mn = [], [], [], []
        for p in ps:
            xs = sorted(x / 1e6 for x in p['c']['wall_ns'][100:500])
            q10.append(xs[int(0.1 * len(xs))])
            q50.append(statistics.median(xs))
            q90.append(xs[int(0.9 * len(xs))])
            mn.append(sum(xs) / len(xs))
        o(f'  W{W:<2d} {name}: p10 {statistics.median(q10):.4f} p50 {statistics.median(q50):.4f} p90 {statistics.median(q90):.4f} mean {statistics.median(mn):.4f}')

o('\npower plan lines in the window log (first and resumed run):')
seen = set()
for ln in open(C.os.path.join(C.RAW, 'window_log.txt'), encoding='utf-8'):
    if 'powercfg' in ln:
        g = ln.split('powercfg')[1].strip()[:90]
        if g not in seen:
            seen.add(g)
            o('  ' + ln[:40] + ' ... ' + g)
o.save('c3_drift.txt')
