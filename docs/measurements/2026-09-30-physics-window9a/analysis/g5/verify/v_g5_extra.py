"""Extra checks: receipt medians under several populations; armed-wall statistic variants; C4-AB C4-JD#tip W8 T(8);
TreeDiag wide/excluded rows on Tree rows; R4 record verdicts."""
import json, os, statistics, sys
sys.argv = ['x']
here = os.path.dirname(os.path.abspath(__file__))
exec(open(os.path.join(here, 'v_g5.py'), encoding='utf-8').read().split("\nP(\"\n=== B1")[0])
O = []
def Q(s=''):
    O.append(str(s)); print(s)
Q('--- receipts: populations ---')
pops = {
    'used 90': used,
    'all 95 timed': allp,
}
for name, pop in pops.items():
    for q in (0, 1, 2):
        u = [p for p in pop if p['pass_'] == q]
        b0 = statistics.median(p['rec']['receipt_before']['cpu_avg'] for p in u)
        a0 = statistics.median(p['rec']['receipt_after']['cpu_avg'] for p in u)
        w0 = statistics.median(p['rec']['others_busy_pct'] for p in u)
        Q(f"{name} p{q}: before {b0:.2f} after {a0:.2f} witness {w0:.2f} (n {len(u)})")
wr = [r for r in recs if r.get('block') == 'C4-G5' and ('row' in r)]
for q in (0, 1, 2):
    u = [r for r in wr if r['pass'] == q]
    b0 = statistics.median(r['receipt_before']['cpu_avg'] for r in u)
    a0 = statistics.median(r['receipt_after']['cpu_avg'] for r in u)
    w0 = statistics.median(r['others_busy_pct'] for r in u)
    Q(f"all incl warm-up p{q}: before {b0:.2f} after {a0:.2f} witness {w0:.2f} (n {len(u)})")
Q('--- armed wall variants (JD-armed, JDap-armed) ---')
for row in ('C4-JD-armed', 'C4-JDap-armed'):
    for Wx in (1, 8):
        us = [p for p in used if p['row'] == row and p['W'] == Wx]
        v1 = statistics.median(statistics.median(p['c']['wall_ns'][100:500]) for p in us) / 1e6
        v2 = statistics.median(statistics.fmean(p['c']['wall_ns'][100:500]) for p in us) / 1e6
        v3 = statistics.median(statistics.median(p['c']['wall_ns'][0:500]) for p in us) / 1e6
        sy = statistics.median(statistics.median(p['c'][SYS][100:500]) for p in us) / 1e6
        Q(f"{row} W{Wx}: median-over-steps wall [100,500) {v1:.4f}; mean [100,500) {v2:.4f}; median [0,500) {v3:.4f}; sys/median-wall {100*sy/v1:.1f}%, sys/mean-wall {100*sy/v2:.1f}%")
Q('--- TreeDiag on all Tree-row processes ---')
bad = []
for p in allp:
    if ROWS[p['row']]['broadphase'] == 'Tree':
        bt = p['s']['broadphase_tree']
        for k in ('wide_rows', 'excluded_rows', 'sleeper_rebuilds', 'translations', 'hint_candidates', 'evictions'):
            if bt.get(k) != 0:
                bad.append((p['row'], p['seq'], k, bt.get(k)))
Q(f"nonzero wide/excluded/sleeper/translations/hint/evictions: {bad}")
Q(f"TreeDiag per Tree row: " + str(sorted(set((p['row'], p['s']['broadphase_tree']['static_rebuilds'], p['s']['broadphase_tree']['members'], p['s']['broadphase_tree']['leaf_list_leaves']) for p in allp if ROWS[p['row']]['broadphase'] == 'Tree'))))
Q(f"AllPairs rows broadphase_tree: " + str(sorted(set(json.dumps(p['s'].get('broadphase_tree'), sort_keys=True)[:120] for p in allp if ROWS[p['row']]['broadphase'] == 'AllPairs'))))
r4 = [r for r in recs if r.get('block') == 'C4-G5' and 'r4' in r]
Q(f"R4 verdicts: {[r['verdict'] for r in r4]}")
Q('--- C4-AB C4-JD#tip W8 T(8) [0,500) mean wall, slot rule, pass_done passes ---')
done_ab = {(r['run_tag'], r['pass'], r['pass_attempt']) for r in recs if r.get('block') == 'C4-AB' and r.get('pass_done')}
ab = [r for r in recs if r.get('block') == 'C4-AB' and r.get('row') == 'C4-JD' and r.get('binary') == 'tip' and r.get('W') == 8 and r.get('attempt') != 'warmup']
sl = {}
for r in ab:
    if (r['run_tag'], r['pass'], r['pass_attempt']) in done_ab:
        sl.setdefault((r['pass'], r['seq']), []).append(r)
vals8 = []
for k, rs in sorted(sl.items()):
    orig = [r for r in rs if r['attempt'] == 'original']
    rer = sorted([r for r in rs if r['attempt'] == 'rerun'], key=lambda r: r['rerun_no'])
    pick = next((r for r in orig + rer if r.get('valid') and not clean_why(r)), None)
    if pick:
        c = load_csv(os.path.join(pick['cwd'], 'run.csv'))
        vals8.append((k[0], statistics.fmean(c['wall_ns'][0:500]) / 1e6, statistics.fmean(c['wall_ns'][100:500]) / 1e6))
Q(f"C4-JD#tip W8 used: {len(vals8)} per pass {[sum(1 for v in vals8 if v[0]==q) for q in (0,1,2)]}; median [0,500) {statistics.median(v[1] for v in vals8):.4f}; [100,500) {statistics.median(v[2] for v in vals8):.4f}")
T8 = statistics.median(v[1] for v in vals8)
for tq in (0.2046, 0.2076):
    Q(f"t_q {tq}: share of T(8) {100*tq/T8:.2f}%; S5 formula tq*0.745-0.01 = {tq*0.745-0.01:.4f} ({100*(tq*0.745-0.01)/T8:.2f}%); E=0.6: {tq*(1-1/4.8)-0.01:.4f}; perfect 8-way tq*7/8 {tq*7/8:.4f}")
open(os.path.join(here, 'v_g5_extra.txt'), 'w', encoding='utf-8').write('\n'.join(O) + '\n')
