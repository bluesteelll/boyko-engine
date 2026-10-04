"""Secondary checks of the analyst's prose (adversarial verifier): estimates.json mean vs recomputed mean,
9a/8b ratios from win8b g4.json, tree ns/row range, and 8b kd/tree rows the F3 reading cites or omits."""
import json, os, statistics, sys
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
SCR = os.path.dirname(W)
OUT = []
def P(s=''):
    OUT.append(str(s)); print(s)
recs = [json.loads(l) for l in open(os.path.join(W, 'raw', 'runs.jsonl'), encoding='utf-8') if l.strip()]
procs = [r for r in recs if r.get('block') in ('C4-G4', 'C4-G4-kd') and 'row' in r]
worst = 0.0
for r in procs:
    for root, dirs, files in os.walk(os.path.join(r['cwd'], 'criterion')):
        if os.path.basename(root) == 'new' and 'estimates.json' in files:
            s = json.load(open(os.path.join(root, 'sample.json')))
            m = statistics.fmean(t / n for t, n in zip(s['times'], s['iters']))
            e = json.load(open(os.path.join(root, 'estimates.json')))['mean']['point_estimate']
            worst = max(worst, abs(e / m - 1))
P(f'estimates.json mean.point_estimate vs recomputed Flat mean, worst rel diff over all ids of 12 processes: {worst:.2e}')
G = {}
for r in procs:
    if r['block'] != 'C4-G4':
        continue
    for root, dirs, files in os.walk(os.path.join(r['cwd'], 'criterion')):
        if os.path.basename(root) == 'new' and 'sample.json' in files:
            fid = json.load(open(os.path.join(root, 'benchmark.json')))['full_id']
            s = json.load(open(os.path.join(root, 'sample.json')))
            G.setdefault(fid, []).append(statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3)
med = {k: statistics.median(v) for k, v in G.items()}
g8 = json.load(open(os.path.join(SCR, 'win8b', 'analysis', 'f3', 'g4.json'), encoding='utf-8'))['cells']
rat = {'all_pairs': [], 'tree': []}
for fam in ('uniform', 'disparity'):
    for arm in ('all_pairs', 'tree'):
        for n in (128, 136, 144, 152, 160):
            i = f'bp_g4_{fam}/{arm}/{n}'
            c8 = g8[i]
            m8 = c8.get('median')
            rat[arm].append((i, med[i] / m8))
for arm, xs in rat.items():
    P(f'9a/8b {arm}: min {min(x[1] for x in xs):.4f} max {max(x[1] for x in xs):.4f} :: ' + ', '.join(f'{i.split("/")[0][6:]}/{i.split("/")[2]} {v:.4f}' for i, v in xs))
for fam, extra in (('uniform', 0), ('disparity', 4)):
    per_row = [med[f'bp_g4_{fam}/tree/{n}'] * 1e3 / (n + extra) for n in range(96, 161, 8)]
    P(f'tree ns/row {fam}: min {min(per_row):.2f} max {max(per_row):.2f} :: ' + ' '.join(f'{v:.2f}' for v in per_row))
    per_pair = [med[f'bp_g4_{fam}/all_pairs/{n}'] * 1e3 / ((n + extra) * (n + extra - 1) / 2) for n in range(96, 161, 8)]
    P(f'all_pairs ns/pair-test {fam}: ' + ' '.join(f'{v:.4f}' for v in per_pair))
open(os.path.join(HERE, 'v2_secondary.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
