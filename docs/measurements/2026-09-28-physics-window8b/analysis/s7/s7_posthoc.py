"""S7-AB POST HOC readings (none pre-registered): T(16)/T(8), P(16)/P(8), J-A P(16) vs T(16), the optional W1 pair,
paired per-round T/P ratios, per-manifold context, sensitivity to the two hot originals, placement receipts.
Reads s7_procs.json. Writes s7_posthoc.txt and s7_posthoc.json."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s7lib as L  # noqa: E402

ALL = json.load(open(os.path.join(L.HERE, 's7_procs.json'), encoding='utf-8'))['procs']
USED = [p for p in ALL if p.get('used')]
PASSES = (0, 1, 2)
WINDOWS = ('0..500', '0..100', '100..500')
OUT = []
RES = {}


def P(s=''):
    OUT.append(s)
    print(s)


def cellof(procs, row, b, W, win, passes=PASSES, f=None):
    f = f or (lambda p: p['wall_ms'][win])
    return L.cell([f(p) for p in procs if p['row'] == row and p['binary'] == b and p['W'] == W and p['pass'] in passes])


def judge(procs, a, b, win, f=None):
    ap, bp = cellof(procs, *a, win, f=f), cellof(procs, *b, win, f=f)
    apass = {q: cellof(procs, *a, win, (q,), f=f) for q in PASSES}
    bpass = {q: cellof(procs, *b, win, (q,), f=f) for q in PASSES}
    j = L.ruling1(ap, bp, apass, bpass)
    j['A_cell'], j['B_cell'] = ap, bp
    return j


def line(tag, j):
    c = j['pooled']
    per = ' | '.join(f"p{q} {cq['ratio']:.4f} {L.yn(cq)}" for q, cq in j['per'].items())
    st = ('CLAIMED' + (' STRONG' if j['strong_all'] else '')) if j['claimed'] else 'NOT CLAIMED'
    P(f"  {tag}: A {L.fcell(j['A_cell'])}; B {L.fcell(j['B_cell'])}")
    thr = max(max(x['bar_i'], x['bar_s']) for x in [c] + list(j['per'].values()))
    P(f"      B/A pooled {L.fcmp(c)}; {per}; ruling-1 reading: {st} ({j['direction']}); binding threshold "
      f"{100 * thr:.2f} % = {thr * j['A_cell']['median']:.4f} ms")
    return {'ratio': c['ratio'], 'delta': c['delta'], 'status': st, 'direction': j['direction'], 'thr_rel': thr,
            'thr_ms': thr * j['A_cell']['median'], 'flags': L.yn(c),
            'per_flags': {str(q): L.yn(cq) for q, cq in j['per'].items()},
            'per_ratio': {str(q): cq['ratio'] for q, cq in j['per'].items()},
            'A': j['A_cell']['median'], 'B': j['B_cell']['median'], 'KA': j['A_cell']['K'], 'KB': j['B_cell']['K']}


PAIRS = {
    'T16/T8 J-T': (('S7-JT', 's7t', 8), ('S7-JT', 's7t', 16)),
    'P16/P8 J-T': (('S7-JT', 's7p', 8), ('S7-JT', 's7p', 16)),
    'T16/T8 J-A': (('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16)),
    'P16/P8 J-A': (('S7-JA', 's7p', 8), ('S7-JA', 's7p', 16)),
    'T16/P16 J-A': (('S7-JA', 's7p', 16), ('S7-JA', 's7t', 16)),
    'T1/P1 J-T (S7-JT-W1, optional Q10 pair)': (('S7-JT-W1', 's7p', 1), ('S7-JT-W1', 's7t', 1)),
}
RES['ratios'] = {}
for win in WINDOWS:
    P(f'\n# POST HOC ratios, window [{win}) (B/A; the ruling-1 reading is descriptive, nothing here was pre-registered)')
    RES['ratios'][win] = {}
    for tag, (a, b) in PAIRS.items():
        RES['ratios'][win][tag] = line(tag, judge(USED, a, b, win))

P('\n# POST HOC scaling T(1)/T(W) on J-T from the W1 pair row (same binary, [0,500))')
for b in ('s7p', 's7t'):
    t1 = cellof(USED, 'S7-JT-W1', b, 1, '0..500')['median']
    s = [f"W{W} {t1 / cellof(USED, 'S7-JT', b, W, '0..500')['median']:.3f}" for W in (8, 16)]
    P(f'  {b}: T(1) {t1:.4f} ms; T(1)/T(W): ' + '; '.join(s))

P('\n# POST HOC per manifold, [100,500): ns per manifold = wall / (each process own manifold mean); counts are identical '
  'in every process of both scenes (s7_select.txt), so every ratio equals the wall ratio')
for row, b, W in (('S7-JT', 's7p', 8), ('S7-JT', 's7t', 8), ('S7-JT', 's7p', 16), ('S7-JT', 's7t', 16),
                  ('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8), ('S7-JA', 's7p', 16), ('S7-JA', 's7t', 16)):
    c = cellof(USED, row, b, W, '100..500', f=lambda p: p['wall_ms']['100..500'] / p['manifolds']['100..500'] * 1e6)
    P(f'  {row}#{b}@W{W}: {L.fcell(c, 1)} ns/manifold')

P('\n# POST HOC paired per round: T/P of the two adjacent processes of one (pass, round), [0,500); n = 9 pairs')
RES['paired'] = {}
for row, W in (('S7-JT', 8), ('S7-JT', 16), ('S7-JA', 8), ('S7-JA', 16), ('S7-JT-W1', 1)):
    rs, ds = [], []
    for q in PASSES:
        for rd in (0, 1, 2):
            pp = [p for p in USED if p['row'] == row and p['W'] == W and p['pass'] == q and p['round'] == rd]
            t = [p for p in pp if p['binary'] == 's7t'][0]['wall_ms']['0..500']
            pv = [p for p in pp if p['binary'] == 's7p'][0]['wall_ms']['0..500']
            rs.append(t / pv)
            ds.append(t - pv)
    RES['paired'][f'{row}@W{W}'] = {'ratios': rs, 'median': statistics.median(rs)}
    P(f'  {row}@W{W}: T/P median {statistics.median(rs):.4f} [{min(rs):.4f}-{max(rs):.4f}], T slower in '
      f'{sum(1 for x in rs if x > 1)} of {len(rs)}; T-P median {statistics.median(ds):+.4f} ms '
      f'[{min(ds):+.4f}..{max(ds):+.4f}]')

P('\n# POST HOC sensitivity: the two hot originals used in place of their clean re-runs (the protocol uses the re-runs)')
HOT = [p for p in ALL if p['attempt'] == 'original' and p['clean_why']]
for h in HOT:
    rer = [p for p in ALL if p['attempt'] == 'rerun' and (p['pass'], p['round'], p['row'], p['binary'], p['W']) ==
           (h['pass'], h['round'], h['row'], h['binary'], h['W'])][0]
    P(f"  p{h['pass']} r{h['round']} {h['row']}#{h['binary']}@W{h['W']}: original {h['wall_ms']['0..500']:.4f} ms "
      f"({h['clean_why']}), re-run {rer['wall_ms']['0..500']:.4f} ms")
ALT = [p for p in ALL if p.get('used') and p['attempt'] != 'rerun'] + HOT
for tag, a, b in (('C2-JA with the hot original', ('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8)),
                  ('C1-JA with the hot original', ('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16)),
                  ('W1 pair with the hot original', ('S7-JT-W1', 's7p', 1), ('S7-JT-W1', 's7t', 1))):
    line(tag, judge(ALT, a, b, '0..500'))

P('\n# Placement receipts (RECORDED, never used to select): per cell, median [min-max] of the main-thread cycle '
  'fraction and of the top logical CPU busy %; then every process more than 1.5 % from its cell median, [0,500)')
cells = sorted({(p['row'], p['binary'], p['W']) for p in USED})
for k in cells:
    ps = [p for p in USED if (p['row'], p['binary'], p['W']) == k]
    mf = [p['placement']['main_cycle_frac'] for p in ps]
    tb = [p['placement']['top3'][0][1] for p in ps]
    P(f'  {k[0]}#{k[1]}@W{k[2]} (n {len(ps)}): main_cycle_frac {statistics.median(mf):.3f} [{min(mf):.3f}-{max(mf):.3f}]; '
      f'top CPU busy {statistics.median(tb):.1f} [{min(tb):.1f}-{max(tb):.1f}] %')
P('  outliers (> 1.5 % from the cell median):')
n_out = 0
for k in cells:
    ps = [p for p in USED if (p['row'], p['binary'], p['W']) == k]
    med = statistics.median(p['wall_ms']['0..500'] for p in ps)
    for p in ps:
        dev = p['wall_ms']['0..500'] / med - 1
        if abs(dev) > 0.015:
            n_out += 1
            pl = p['placement']
            P(f"    p{p['pass']} r{p['round']} {k[0]}#{k[1]}@W{k[2]} {p['start'][11:19]}: {p['wall_ms']['0..500']:.4f} ms "
              f"({100 * dev:+.2f} %); main_cycle_frac {pl['main_cycle_frac']}, top3 {pl['top3']}, "
              f"receipts {p['receipt_before']}/{p['receipt_after']} %, witness {p['others_busy_pct']} %")
P(f'  {n_out} outlier processes of {len(USED)}')

open(os.path.join(L.HERE, 's7_posthoc.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
json.dump(RES, open(os.path.join(L.HERE, 's7_posthoc.json'), 'w', encoding='utf-8'), indent=1)

# appended: launch order inside each adjacent P/T pair, per pass (the reversal is the protocol's order control)
P('\n# POST HOC order inside the adjacent pair (by seq; re-runs excluded), T/P per pass')
for row, W in (('S7-JT', 8), ('S7-JT', 16), ('S7-JA', 8), ('S7-JA', 16), ('S7-JT-W1', 1)):
    for q in PASSES:
        firsts, rr = [], []
        for rd in (0, 1, 2):
            pp = [p for p in ALL if p['row'] == row and p['W'] == W and p['pass'] == q and p['round'] == rd
                  and p['attempt'] == 'original']
            pp.sort(key=lambda p: p['seq'])
            firsts.append(pp[0]['binary'])
            u = {p['binary']: p for p in USED if p['row'] == row and p['W'] == W and p['pass'] == q and p['round'] == rd}
            rr.append(u['s7t']['wall_ms']['0..500'] / u['s7p']['wall_ms']['0..500'])
        P(f"  {row}@W{W} p{q}: first launched {firsts}; T/P per round {', '.join(f'{x:.4f}' for x in rr)}")
open(os.path.join(L.HERE, 's7_posthoc.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
