"""F3-G4 recomputed from criterion sample.json (Flat: mean of time/iters), K = 3, one pass (independent)."""
import json
import math
import os
import re
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

recs = L.load_recs()
g4 = [r for r in recs if r.get('block') == 'F3-G4' and r.get('kind') == 'criterion']
FAM = ('uniform', 'disparity')
SIZES = (128, 136, 144, 152, 160, 176, 256, 1000)
ids = [f'bp_g4_{f}/{a}/{n}' for f in FAM for a in ('all_pairs', 'tree', 'tree_kd') for n in SIZES]
ids += [f'bp_g4_{f}/tree_rowwalk/{n}' for f in FAM for n in (144, 256)]
ids += [f'bp_g4_scene/{a}/{n}' for a in ('tree', 'tree_kd') for n in ('j100', '1240', '10000', '100000')]
assert len(ids) == 60
data = {i: [] for i in ids}
for r in g4:
    d = r['cwd']
    so = L.rtext(os.path.join(d, 'stdout.txt'))
    printed = re.findall(r'^(bp_g4_\S+)', so, re.M)
    why = []
    if r.get('exit') != 0:
        why.append('exit')
    if r.get('exe_sha256') != L.G4_SHA:
        why.append('sha')
    if sorted(printed) != sorted(ids):
        why.append(f'ids printed {len(printed)} set-equal {set(printed) == set(ids)}')
    ntime = len(re.findall(r'time:\s+\[', so))
    if ntime != 60:
        why.append(f'time lines {ntime}')
    for i in ids:
        sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
        sm = json.load(open(sp))
        if sm['sampling_mode'] != 'Flat' or len(sm['times']) != 10:
            why.append(f'{i} mode')
        data[i].append(sum(t / n for t, n in zip(sm['times'], sm['iters'])) / len(sm['times']) / 1e3)  # us
    print(f"G4 p{r['pass']} r{r['round']} attempt {r['attempt']}: valid {not why} {why} clean_why {L.clean_why(r)} "
          f"rb {r['receipt_before']['cpu_avg']} ra {r['receipt_after']['cpu_avg']} witness {r['others_busy_pct']}")
print('processes:', len(g4))
C = {i: L.cell(v) for i, v in data.items()}


def is_(c, rule):
    if rule == 'ruling1':
        return c['fl']['i'] and c['fl']['s']
    return c['fl']['r'] and c['fl']['s']  # window 7


def table(arm):
    res = {}
    for f in FAM:
        print(f'  all_pairs / {arm}, {f} (ratio > 1: {arm} faster); flags r/i/s')
        for n in SIZES:
            a = C[f'bp_g4_{f}/{arm}/{n}']
            b = C[f'bp_g4_{f}/all_pairs/{n}']
            c = L.cmp(a, b)
            res[(f, n)] = c
            print(f"    {n:5d}: {arm} {a['m']:.3f} us, all_pairs {b['m']:.3f} us, ratio {c['ratio']:.4f} "
                  f"({100*(c['ratio']-1):+.2f} %) {L.yn(c)} bars r {100*c['bars']['r']:.2f} i {100*c['bars']['i']:.2f} s {100*c['bars']['s']:.2f}")
    return res


def lohi(res, rule):
    out = {}
    for f in FAM:
        faster = [n for n in SIZES if res[(f, n)]['ratio'] > 1 and is_(res[(f, n)], rule)]
        notf = [n for n in SIZES if not (res[(f, n)]['ratio'] > 1 and is_(res[(f, n)], rule))]
        hi = min(faster) if faster else None
        lo = max(notf) if notf else None  # largest n at which all_pairs is NOT claimed slower
        out[f] = (lo, hi, faster)
    return out


def crossover(arm):
    out = {}
    for f in FAM:
        xs = [(n, math.log(C[f'bp_g4_{f}/all_pairs/{n}']['m'] / C[f'bp_g4_{f}/{arm}/{n}']['m'])) for n in SIZES]
        for (n0, y0), (n1, y1) in zip(xs, xs[1:]):
            if y0 <= 0 < y1:
                t = -y0 / (y1 - y0)
                out[f] = math.exp(math.log(n0) + t * (math.log(n1) - math.log(n0)))
                break
        else:
            out[f] = None if xs[0][1] <= 0 else f'below {SIZES[0]} (ratio at 128 already > 1)'
    return out


for arm in ('tree', 'tree_kd'):
    print(f'=== {arm}')
    res = table(arm)
    for rule in ('ruling1', 'w7'):
        print(f'   LO/HI under {rule}:', lohi(res, rule))
    print('   log-log crossover:', crossover(arm))
    if arm == 'tree':
        for f in FAM:
            c144, c152 = res[(f, 144)], res[(f, 152)]
            print(f"   threshold legs {f}: 144 all_pairs claimed slower? ruling1 {c144['ratio'] > 1 and is_(c144, 'ruling1')} "
                  f"w7 {c144['ratio'] > 1 and is_(c144, 'w7')}; 152 tree claimed faster? ruling1 {c152['ratio'] > 1 and is_(c152, 'ruling1')} "
                  f"w7 {c152['ratio'] > 1 and is_(c152, 'w7')}")
print('=== bp_g4_scene tree_kd / tree (ratio < 1: kd faster)')
for n in ('j100', '1240', '10000', '100000'):
    a, b = C[f'bp_g4_scene/tree/{n}'], C[f'bp_g4_scene/tree_kd/{n}']
    c = L.cmp(a, b)
    print(f"   {n:>6s}: tree {a['m']:.2f} [{a['min']:.2f}-{a['max']:.2f}] kd {b['m']:.2f} [{b['min']:.2f}-{b['max']:.2f}] us "
          f"ratio {c['ratio']:.4f} {L.yn(c)} bars r {100*c['bars']['r']:.2f} i {100*c['bars']['i']:.2f} s {100*c['bars']['s']:.2f}"
          f"  kd slower claimed(ruling1)? {c['ratio'] > 1 and c['fl']['i'] and c['fl']['s']}")
    print(f"          tree values {[round(x, 2) for x in a['v']]} kd values {[round(x, 2) for x in b['v']]}")
print('=== tree_rowwalk / tree at 144, 256')
for f in FAM:
    for n in (144, 256):
        a, b = C[f'bp_g4_{f}/tree/{n}'], C[f'bp_g4_{f}/tree_rowwalk/{n}']
        c = L.cmp(a, b)
        print(f"   {f} {n}: rowwalk/tree {c['ratio']:.4f} {L.yn(c)}")
json.dump({i: C[i] for i in ids}, open(os.path.join(L.HERE, 'v_g4.json'), 'w'), indent=0)
