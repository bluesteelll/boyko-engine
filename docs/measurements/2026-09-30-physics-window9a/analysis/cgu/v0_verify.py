"""Second derivation of the C4-CGU decisive numbers WITHOUT lib9a / cgu_lib: raw/runs.jsonl + each process's run.csv
read with the csv module; slot rule and clean rule re-implemented from PREP.md's text; statistics re-implemented.
Prints, per W and window: n per pass, pooled medians, B/A, flags i/s/r pooled and per pass, verdict."""
import csv
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.dirname(os.path.dirname(HERE))
recs = [json.loads(x) for x in open(os.path.join(W9A, 'raw', 'runs.jsonl'), encoding='utf-8') if x.strip()]
closed = {(r['pass'], r['pass_attempt']) for r in recs if r.get('pass_done') and r.get('block') == 'C4-CGU'}
P = [r for r in recs if r.get('block') == 'C4-CGU' and 'row' in r and r['attempt'] != 'warmup'
     and (r['pass'], r['pass_attempt']) in closed]
PIN = {'tip': '8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3',
       'tipcgu1': '3e1f72babf76987260988481f12ddbe409ecdc308ce141326529ef89b020d771'}


def clean(r):
    rb, ra = r['receipt_before'], r['receipt_after']
    return (rb['cpu_avg'] <= 5 and ra['cpu_avg'] <= 5 and r['others_busy_pct'] <= 2 and not r['build_proc_during']
            and not rb['build_procs_busy'] and not ra['build_procs_busy']
            and not any(rb['presence'].values()) and not any(ra['presence'].values()))


def valid(r):
    return r['exit'] == 0 and r['exe_sha256'] == PIN[r['binary']] and r['summary']['expect_pose'] == 'match'


def wall(r):
    rows = list(csv.DictReader(open(os.path.join(r['cwd'], 'run.csv'), encoding='utf-8', newline='')))
    assert len(rows) == 500
    return [float(x['wall_ns']) for x in rows]


slots = {}
for r in P:
    slots.setdefault((r['pass'], r['seq']), []).append(r)
used = []
for k, rs in slots.items():
    rs = sorted(rs, key=lambda r: (r['attempt'] != 'original', r.get('rerun_no', 0)))
    pick = next((r for r in rs if valid(r) and clean(r)), None)
    if pick:
        used.append(pick)
print(f'slots {len(slots)} used {len(used)}')


def stats(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return m, (q[2] - q[0]) / m, 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) / m, (xs[-1] - xs[0]) / m


def flags(a, b):
    ma, ia, sa, ra = stats(a)
    mb, ib, sb, rb = stats(b)
    e = abs(mb / ma - 1)
    return mb / ma, ''.join('Y' if e > 2 * math.hypot(x, y) else 'n' for x, y in ((ia, ib), (sa, sb), (ra, rb)))


for (lo, hi) in ((100, 500), (0, 500)):
    print(f'window [{lo},{hi})')
    for W in (1, 2, 4, 8, 16):
        v = {}
        for r in used:
            if r['W'] == W:
                ws = wall(r)[lo:hi]
                v.setdefault((r['binary'], r['pass']), []).append(sum(ws) / len(ws) / 1e6)
        A = [x for k in (0, 1, 2) for x in v[('tip', k)]]
        B = [x for k in (0, 1, 2) for x in v[('tipcgu1', k)]]
        rat, fl = flags(A, B)
        per = [flags(v[('tip', k)], v[('tipcgu1', k)]) for k in (0, 1, 2)]
        ns = '/'.join(f"{len(v[('tip', k)])}+{len(v[('tipcgu1', k)])}" for k in (0, 1, 2))
        cl = fl[:2] == 'YY' and all(f[:2] == 'YY' for _, f in per) and len({x > 1 for x, _ in per} | {rat > 1}) == 1
        print(f'  W{W}: n {ns}; A {statistics.median(A):.4f} B {statistics.median(B):.4f} B/A {rat:.4f} pooled {fl} '
              f'per pass {[f for _, f in per]} -> {"CLAIMED" if cl else "NOT CLAIMED"}')
