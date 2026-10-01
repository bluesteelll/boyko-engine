"""Post hoc G4 checks: tree_kd/tree per size, and 8b against window 7 wave 2 Q3 recomputed from its raw sample.json."""
import glob
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

C8 = json.load(open(os.path.join(L.HERE, 'v_g4.json')))
FAM = ('uniform', 'disparity')
SIZES = (128, 136, 144, 152, 160, 176, 256, 1000)
print('tree_kd / tree per size (ratio > 1: kd slower); flags r/i/s')
for f in FAM:
    line = []
    for n in SIZES:
        c = L.cmp(C8[f'bp_g4_{f}/tree/{n}'], C8[f'bp_g4_{f}/tree_kd/{n}'])
        line.append(f"{n} {c['ratio']:.4f} {L.yn(c)}")
    print('  ', f, '; '.join(line))
W7 = ('C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/'
      'w8s-s4/wt-dead-0928/docs/measurements/2026-09-25-physics-window7/wave2/raw/Q3-p0_060748')
procs = sorted(glob.glob(os.path.join(W7, '*G4-refine*')))
print('window 7 Q3 processes:', len(procs))
w7 = {}
for d in procs:
    for sp in glob.glob(os.path.join(d, 'criterion', 'bp_g4_*', '*', '*', 'new', 'sample.json')):
        parts = sp.replace(chr(92), '/').split('/')
        i = '/'.join(parts[-5:-2])
        sm = json.load(open(sp))
        w7.setdefault(i, []).append(sum(t / k for t, k in zip(sm['times'], sm['iters'])) / len(sm['times']) / 1e3)
print('window 7 ids:', len(w7), 'K per id:', sorted({len(v) for v in w7.values()}))
for arm in ('all_pairs', 'tree', 'tree_rowwalk'):
    rs = []
    for f in FAM:
        for n in SIZES:
            k = f'bp_g4_{f}/{arm}/{n}'
            if k in w7 and k in C8:
                rs.append((f, n, C8[k]['m'] / statistics.median(w7[k])))
    if rs:
        v = [x[2] for x in rs]
        print(f"  {arm}: shared {len(rs)}, 8b/w7 median {statistics.median(v):.4f}, range {min(v):.4f}-{max(v):.4f}")
for f in FAM:
    a = statistics.median(w7[f'bp_g4_{f}/all_pairs/144']) / statistics.median(w7[f'bp_g4_{f}/tree/144'])
    print(f'  window 7 all_pairs/tree at 144 {f}: {a:.4f}')
