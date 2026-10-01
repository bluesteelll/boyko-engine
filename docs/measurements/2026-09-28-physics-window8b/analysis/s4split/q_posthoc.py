"""POST HOC (not pre-registered for S4): (1) R_8 / R_16 of the TIP binary from S7-AB ladder rows (another block,
same file as TIP); (2) window 8 per-wave units: its q3 table divided the raw CSV counters (TSC ticks) as ns;
(3) TIP vs PARENT first-wave / pass ramps in S4-AB (armed)."""
from common_s4 import *
import glob
import q_split as QS  # runs q_split (rewrites q_split.txt identically); its lines are cleared below
OUT.clear()

P('# POST HOC 1: the TIP binary resolution from block S7-AB (s7p = runner_tip_16191fda.exe), ruling 1, [0,500)')
for W, ref, rungs in ((8, 60000, ('0.5', '1', '1.5', '2')), (16, 105000, ('0.5', '1', '1.5', '2'))):
    seen = {}
    for F_ in rungs:
        rid = 'S7-ladder%d-F%s' % (W, F_)
        j = judge('S7-JT', 's7p', rid, 's7p', W, 'S7-AB', wall('0..500'), +1)
        inj = float(F_) * ref / 1e6
        seen[float(F_)] = j['verdict'] == 'CLAIMED'
        P('W%d %s (inj %.4f ms): rise %+.4f ms, pooled %s; passes %s -> %s%s' % (
            W, rid, inj, j['pooled']['delta'], L.yn(j['pooled']),
            ' '.join('p%d %+.4f %s' % (c['pass_'], c['delta'], L.yn(c)) for c in j['per']), j['verdict'],
            ' STRONG' if j['strong'] else ''))
    ks = sorted(seen)
    demo = [k for k in ks if all(seen[q] for q in ks if q >= k)]
    P('  R_%d (smallest rung seen with every larger rung seen): %s' % (
        W, '%.4f ms' % (min(demo) * ref / 1e6) if demo else 'none'))
    ref_c = C('S7-JT', W, 'S7-AB', wall('0..500'), 's7p')
    P('  reference S7-JT#s7p@W%d %s' % (W, L.fc(ref_c)))

P('\n# POST HOC 2: window 8 q3 per-wave numbers were TSC ticks. Recomputed on win8/raw, J-T-a block W8S-A, [100,500)')
W8R = os.path.join(os.path.dirname(L.W8B), 'win8', 'raw')
for W in (8, 16):
    raw_r, raw_t, ns_r, ns_t, npr, npt, tp = [], [], [], [], [], [], []
    for d in sorted(glob.glob(os.path.join(W8R, 'W8S-A-p*', '*_J-T-a_instr_W%d' % W))):
        s = L.summary_of(L.read_text(os.path.join(d, 'stdout.txt')))
        c = L.load_csv(os.path.join(d, 'run.csv'))
        tpn = s['ticks_per_ns']
        sc = L.ssum(c['phys_color_scopes'][100:500])
        npn = sum(1 for v in c['phys_np_wave_ramp_n'][100:500] if v)
        rr = L.ssum(c['phys_wave_ramp'][100:500]) / sc
        tt = L.ssum(c['phys_wave_tail'][100:500]) / sc
        raw_r.append(rr)
        raw_t.append(tt)
        ns_r.append(rr / tpn)
        ns_t.append(tt / tpn)
        npr.append(L.ssum(c['phys_np_wave_ramp'][100:500]) / npn / tpn)
        npt.append(L.ssum(c['phys_np_wave_tail'][100:500]) / npn / tpn)
        tp.append(tpn)
    med = statistics.median
    P('W%d (K=%d): ramp/wave raw %.3f "us" -> %.3f us; tail/wave raw %.3f -> %.3f us; ramp+tail %.3f -> %.3f us; '
      'np ramp %.3f us, np tail %.3f us; ticks_per_ns %.4f-%.4f' % (
          W, len(raw_r), med(raw_r) / 1e3, med(ns_r) / 1e3, med(raw_t) / 1e3, med(ns_t) / 1e3,
          (med(raw_r) + med(raw_t)) / 1e3, (med(ns_r) + med(ns_t)) / 1e3, med(npr) / 1e3, med(npt) / 1e3,
          min(tp), max(tp)))

P('\n# POST HOC 3: S4-AB armed, TIP vs PARENT per-wave fields [100,500) (TIP sums include its one setup wave per step)')
for W in (8, 16):
    for b in ('parent', 'tip'):
        ms = [QS.split(p, '100..500') for p in sel('S4-JT-a', W, 'S4-AB', b)]
        f = lambda k: statistics.median([m[k] for m in ms])
        P('W%d %s (K=%d): waves/step %.2f, first ramp %.1f ns, pass ramp %.1f ns, ramp/wave %.1f, join/wave %.1f, '
          'imbalance/wave %.1f, first tail %.1f, ramp sum %.2f us/step, tail sum %.2f us/step' % (
              W, b, len(ms), f('waves_per_step'), f('first_ramp_ns_mean'), f('pass_ramp_ns_mean'), f('ramp_ns_mean'),
              f('join_ns_mean'), f('imbalance_ns_mean'), f('first_tail_ns_mean'), f('ramp_us_step'), f('tail_us_step')))

P('\n# POST HOC 4: SPLIT-J-T-a (block SPLIT) vs S4-JT-a#parent (block S4-AB), same exe/row/W: wall [0,500) medians')
for W in (8, 16):
    a = C('S4-JT-a', W, 'S4-AB', wall('0..500'), 'parent')
    b = C('SPLIT-J-T-a', W, 'SPLIT', wall('0..500'), 'parent')
    a2 = C('S4-JT', W, 'S4-AB', wall('0..500'), 'parent')
    b2 = C('SPLIT-J-T', W, 'SPLIT', wall('0..500'), 'parent')
    P('W%d armed S4-AB %.4f vs SPLIT %.4f; disarmed S4-AB %.4f vs SPLIT %.4f' % (W, a['median'], b['median'],
                                                                              a2['median'], b2['median']))
a2 = C('S4-JT', 1, 'S4-AB', wall('0..500'), 'parent')
b2 = C('SPLIT-J-T', 1, 'SPLIT', wall('0..500'), 'parent')
P('W1 disarmed S4-AB %.4f vs SPLIT %.4f' % (a2['median'], b2['median']))
save('q_posthoc.txt')

P('\n# POST HOC 2b: window 8 "W16 growth mostly in the tail: +1.66 us/wave x 96 ~ 0.16 ms" in ns (win8/raw, W8S-A J-T-a, [100,500))')
tails = {}
for W in (8, 16):
    v = []
    for d in sorted(glob.glob(os.path.join(W8R, 'W8S-A-p*', '*_J-T-a_instr_W%d' % W))):
        s = L.summary_of(L.read_text(os.path.join(d, 'stdout.txt')))
        c = L.load_csv(os.path.join(d, 'run.csv'))
        sc = L.ssum(c['phys_color_scopes'][100:500])
        v.append((L.ssum(c['phys_wave_tail'][100:500]) / sc / s['ticks_per_ns'], sc / 400.0))
    tails[W] = (statistics.median(x[0] for x in v), statistics.median(x[1] for x in v))
P('tail/wave W8 %.3f us, W16 %.3f us; growth %.3f us/wave x %.0f waves = %.4f ms per step' % (
    tails[8][0] / 1e3, tails[16][0] / 1e3, (tails[16][0] - tails[8][0]) / 1e3, tails[16][1],
    (tails[16][0] - tails[8][0]) * tails[16][1] / 1e6))
save('q_posthoc.txt')

P('\n# POST HOC 2c: window 8 omega(W) in ns from win8/raw (spans are ns in the CSV), W8S-A J-T-a, [100,500)')
wide = {}
wv = {}
for W in (1, 8, 16):
    v, n = [], []
    for d in sorted(glob.glob(os.path.join(W8R, 'W8S-A-p*', '*_J-T-a_instr_W%d' % W))):
        c = L.load_csv(os.path.join(d, 'run.csv'))
        v.append(L.mean(c['phys_color_wide_ns'][100:500]) / 1e3)
        n.append(L.mean(c['phys_color_scopes'][100:500]))
    wide[W] = statistics.median(v)
    wv[W] = statistics.median(n)
for W in (8, 16):
    om = (wide[W] - wide[1] / W) / wv[W]
    P('W%d: wide %.2f us, wide(1) %.2f us, waves %.2f, omega %.3f us; ramp+tail (ns-corrected) / omega = %.1f %%' % (
        W, wide[W], wide[1], wv[W], om, 100 * ((1.625 if W == 8 else 2.166) / om)))
save('q_posthoc.txt')
