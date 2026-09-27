from q2_identity import *

OUT.clear()
P('# Q4 resolution (block W8S-R; reference R-J-T at the same W; the rise must be claimed Y under r AND s, ratio > 1)')
for win in ('0..500', '100..500'):
    P(f'\n## window [{win})')
    for W, ref_ns, rows in ((8, 60000, ['ladder8-F0.5', 'ladder8-F1', 'ladder8-F1.5', 'ladder8-F2']),
                            (16, 105000, ['ladder16-F0.5', 'ladder16-F1', 'ladder16-F1.5', 'ladder16-F2'])):
        ref = C('R-J-T', W, wall(win), ['W8S-R'])
        P(f'W{W} reference R-J-T: {L.fc(ref)}')
        seen = {}
        for rid in rows:
            F_ = float(rid.split('-F')[1])
            c = C(rid, W, wall(win), ['W8S-R'])
            cm = L.cmp_(ref, c)
            ok = cm['claimed'] and cm['ratio'] > 1
            seen[F_] = ok
            inj = F_ * ref_ns / 1e6
            P(f'  {rid}: {L.fc(c)}; rise {cm["delta"]:+.4f} ms (injected {inj:.4f}; rise/inj {cm["delta"] / inj:.3f}) = {100 * (cm["ratio"] - 1):+.2f} %; {L.yn(cm)} bars r {100 * cm["bar_r"]:.2f} / s {100 * cm["bar_s"]:.2f} % -> {"SEEN" if ok else "not seen"}')
            for ps in ((0,), (1,)):
                a = C('R-J-T', W, wall(win), ['W8S-R'], passes=ps)
                b = C(rid, W, wall(win), ['W8S-R'], passes=ps)
                cc = L.cmp_(a, b)
                P(f'      pass {ps[0]} (K=3, diag): rise {cc["delta"]:+.4f} {L.yn(cc)} bars r {100 * cc["bar_r"]:.2f} s {100 * cc["bar_s"]:.2f}')
        ks = sorted(seen)
        demo = [k for k in ks if all(seen[j] for j in ks if j >= k)]
        rk = min(demo) if demo else None
        P(f'  R_{W}: smallest rung seen with every larger rung seen: {rk} x {ref_ns / 1e3:.0f} us = {"%.3f ms" % (rk * ref_ns / 1e6) if rk else "none"}')
    P('\n  rung at W 1/2/4 (0.060 ms, F=1 x 60000):')
    for W in (1, 2, 4):
        ref = C('R-J-T', W, wall(win), ['W8S-R'])
        c = C('rung', W, wall(win), ['W8S-R'])
        cm = L.cmp_(ref, c)
        ok = cm['claimed'] and cm['ratio'] > 1
        P(f'  W{W}: ref {L.fc(ref)}; rung {L.fc(c)}; rise {cm["delta"]:+.4f} ms ({100 * (cm["ratio"] - 1):+.2f} %) {L.yn(cm)} bars r {100 * cm["bar_r"]:.2f} / s {100 * cm["bar_s"]:.2f} % -> {"SEEN" if ok else "NOT SEEN: unresolved above %.2f %%" % (100 * cm["bar_r"])}')
P('\n## zone canary on phys_solve_build (armed, W8; reference R-J-T-a W8)')
for win in ('100..500', '0..500'):
    ref_s = L.cell([p['stats'][win]['phys_solve_build_ns'] / 1e6 for p in sel('R-J-T-a', 8, ['W8S-R'])])
    ref_w = C('R-J-T-a', 8, wall(win), ['W8S-R'])
    P(f'[{win}) ref span {L.fc(ref_s)}; ref wall {L.fc(ref_w)}')
    for rid, N in (('zone-N30000', 30000), ('zone-N60000', 60000)):
        cs = L.cell([p['stats'][win]['phys_solve_build_ns'] / 1e6 for p in sel(rid, 8, ['W8S-R'])])
        cw = C(rid, 8, wall(win), ['W8S-R'])
        ms = L.cmp_(ref_s, cs)
        mw = L.cmp_(ref_w, cw)
        oks = ms['claimed'] and ms['ratio'] > 1
        okw = mw['claimed'] and mw['ratio'] > 1
        P(f'  {rid}: span {L.fc(cs)}; span rise {ms["delta"]:+.4f} ms (inj {N / 1e6:.3f}) {L.yn(ms)} bars r {100 * ms["bar_r"]:.2f} / s {100 * ms["bar_s"]:.2f} % -> {"SEEN" if oks else "not seen"}')
        P(f'      wall {L.fc(cw)}; wall rise {mw["delta"]:+.4f} ms {L.yn(mw)} bars r {100 * mw["bar_r"]:.2f} / s {100 * mw["bar_s"]:.2f} % -> {"SEEN" if okw else "not seen"}')
    # what span effect would the 0.6 x span bar need
P('\n## armed vs disarmed J-T at W8 (block R): wall ' + L.fcmp(L.cmp_(C('R-J-T', 8, wall(), ['W8S-R']), C('R-J-T-a', 8, wall(), ['W8S-R']))))
save('q4.txt')
