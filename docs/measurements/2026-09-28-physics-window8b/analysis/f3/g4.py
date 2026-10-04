"""Block F3-G4 (criterion, K = 3: one pass x three rounds): thresholds on tree and tree_kd (recipe 1.4), the RowWalk
bridge, and R3's scene condition. Estimates recomputed from criterion/<id>/new/sample.json (Flat sampling: the mean
of times/iters over the 10 samples, window 7 wave 2's reader); the printed estimate is compared, not used.
Ruling 1 with one pass: the pass IS the pooled cell, so CLAIMED = i AND s on that cell pair; STRONG = also r.
Window 7 wave 2's rule (r AND s), under which 144/152 were set, is printed beside (column 'w7')."""
import json
import math
import os
import re
import statistics
import sys
from collections import defaultdict

sys.dont_write_bytecode = True
import lib_f3 as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


TIME_RE = re.compile(r'^(bp_g4_\S+)\s*\n?\s*time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]', re.M)
UNIT_US = {'ns': 1e-3, 'ms': 1e3, 's': 1e6, 'ps': 1e-6}
KRE = re.compile(r'^(bp_g4_\w+)/(\w+): kernel (\w+) rows (\d+) pairs (\d+) .*kd_order_builds (\d+)$', re.M)


def unit_us(u):
    if u in UNIT_US:
        return UNIT_US[u]
    if u.endswith('s') and len(u) == 2:
        return 1.0
    raise ValueError(u)


recs = [r for r in L.load_recs() if r.get('block') == 'F3-G4' and 'row' in r]
row = L.ROWS['G4']
need = sorted(row['expect'])
P('F3-G4 records: %d (%s)' % (len(recs), [(r['round'], r['attempt']) for r in recs]))
est = []
for r in sorted(recs, key=lambda x: x['round']):
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append('exit')
    if not L.sha_ok(r):
        why.append('sha')
    so = L.read_text(os.path.join(d, 'stdout.txt'))
    se = L.read_text(os.path.join(d, 'stderr.txt'))
    printed = {}
    for m in TIME_RE.finditer(so):
        printed.setdefault(m.group(1), []).append(float(m.group(4)) * unit_us(m.group(5)))
    e = {}
    for i in need:
        sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
        if not os.path.exists(sp):
            continue
        s = json.load(open(sp, encoding='utf-8'))
        if s.get('sampling_mode') != 'Flat' or len(s['times']) != 10:
            why.append('%s sampling %s x%d' % (i, s.get('sampling_mode'), len(s['times'])))
        e[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3  # ns -> us
    on_disk = set()
    for root, dirs, files in os.walk(os.path.join(d, 'criterion')):
        if os.path.basename(root) == 'new' and 'benchmark.json' in files:
            on_disk.add(json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8'))['full_id'])
    miss = [i for i in need if i not in e or i not in printed]
    extra = sorted((set(printed) | on_disk) - set(need))
    dup = [i for i, v in printed.items() if len(v) != 1]
    if miss:
        why.append('missing %s' % miss[:3])
    if extra:
        why.append('extra %s' % extra[:3])
    if dup:
        why.append('dup %s' % dup[:3])
    worst = max(abs(printed[i][0] / e[i] - 1) for i in e if i in printed)
    if worst > 1e-4:
        why.append('printed vs sample.json %.2e' % worst)
    kr = defaultdict(dict)
    for grp, param, kern, rows_, pairs, kdb in KRE.findall(se):
        kr[(grp, param)][kern] = (int(rows_), int(pairs), int(kdb))
    kbad = []
    for k, v in kr.items():
        if set(v) != {'LeafList', 'RowWalk', 'LeafListKd'} or len({x[1] for x in v.values()}) != 1:
            kbad.append(k)
        elif v['LeafListKd'][2] == 0 or v['LeafList'][2] != 0 or v['RowWalk'][2] != 0:
            kbad.append(k)
    if kbad:
        why.append('kernel receipts %s' % kbad[:3])
    cw = L.clean_why(r)
    P('- round %d %s: ids %d/%d, printed-vs-recomputed worst %.2e, kernel receipts %d (%s), oracle-ok lines %d; '
      'receipts %s/%s %%, witness %s %% -> valid %s, clean %s; %s .. %s' % (
          r['round'], r['attempt'], len(e), len(need), worst, len(kr), 'OK' if not kbad else 'BAD',
          se.count('oracle ok'), (r.get('receipt_before') or {}).get('cpu_avg'),
          (r.get('receipt_after') or {}).get('cpu_avg'), r.get('others_busy_pct'), not why, not cw, r.get('start'),
          r.get('end')))
    if why or cw:
        P('    problems: %s %s' % (why, cw))
    if not why and not cw:
        est.append(e)
P('used processes: %d' % len(est))


def C(i):
    return L.cell([e[i] for e in est])


def claimed(c):
    return c['cl_i'] and c['cl_s']


def w7(c):
    return c['cl_r'] and c['cl_s']


SIZES = [128, 136, 144, 152, 160, 176, 256, 1000]
RES = {'families': {}, 'cells': {i: C(i) for i in need}}
for fam in ('uniform', 'disparity'):
    P('')
    P('## %s' % fam)
    RES['families'][fam] = {}
    for arm in ('tree', 'tree_kd'):
        per = []
        P('### all_pairs / %s (ratio > 1: %s faster; flags r/i/s; ruling-1 claim = i AND s)' % (arm, arm))
        for n in SIZES:
            a = C('bp_g4_%s/all_pairs/%d' % (fam, n))
            t = C('bp_g4_%s/%s/%d' % (fam, arm, n))
            c = L.cmp_(t, a)
            per.append((n, c))
            tag = ''
            if claimed(c):
                tag = '  <- %s CLAIMED faster' % arm if c['ratio'] > 1 else '  <- all_pairs CLAIMED faster'
            P('- n=%-4d all_pairs %s | %s %s | all_pairs/%s %s%s [w7 rule: %s]' % (
                n, L.fcell(a, 1, 3), arm, L.fcell(t, 1, 3), arm, L.fcmp(c), tag,
                'claimed' if w7(c) else 'not claimed'))
        out = {}
        for rule, f in (('ruling1', claimed), ('w7', w7)):
            s = [(n, f(c) and c['ratio'] > 1) for n, c in per]
            lo = max((n for n, x in s if not x), default=None)
            hi = min((n for n, x in s if x), default=None)
            mono = hi is not None and all((n >= hi) == x for n, x in s)
            out[rule] = {'LO': lo, 'HI': hi, 'monotone': mono}
        cross = None
        for (n0, c0), (n1, c1) in zip(per, per[1:]):
            if c0['ratio'] <= 1 and c1['ratio'] > 1:
                y0, y1 = math.log(c0['ratio']), math.log(c1['ratio'])
                cross = math.exp(math.log(n0) - y0 * (math.log(n1) - math.log(n0)) / (y1 - y0))
                break
        out['loglog'] = cross
        out['cmp'] = {n: c for n, c in per}
        RES['families'][fam][arm] = out
        P('  => %s %s: ruling 1 LO %s / HI %s (monotone %s); window-7 rule LO %s / HI %s (monotone %s); log-log '
          'crossover %s' % (fam, arm, out['ruling1']['LO'], out['ruling1']['HI'], out['ruling1']['monotone'],
                            out['w7']['LO'], out['w7']['HI'], out['w7']['monotone'],
                            '%.1f' % cross if cross else 'none in grid'))
    P('### tree_kd against tree (post hoc except where a rule names it; ratio < 1: tree_kd faster)')
    for n in SIZES:
        t = C('bp_g4_%s/tree/%d' % (fam, n))
        k = C('bp_g4_%s/tree_kd/%d' % (fam, n))
        c = L.cmp_(t, k)
        P('- n=%-4d tree_kd/tree %s%s' % (n, L.fcmp(c), '  CLAIMED' if claimed(c) else ''))
    P('### bridge: tree (LeafList) against tree_rowwalk (ratio < 1: LeafList faster)')
    for n in (144, 256):
        rw = C('bp_g4_%s/tree_rowwalk/%d' % (fam, n))
        t = C('bp_g4_%s/tree/%d' % (fam, n))
        k = C('bp_g4_%s/tree_kd/%d' % (fam, n))
        a = C('bp_g4_%s/all_pairs/%d' % (fam, n))
        P('- n=%d tree_rowwalk %s; LeafList/RowWalk %s; tree_kd/RowWalk %s; all_pairs/tree_rowwalk %.3f' % (
            n, L.fcell(rw, 1, 3), L.fcmp(L.cmp_(rw, t)), L.fcmp(L.cmp_(rw, k)), a['median'] / rw['median']))
P('')
P('## bp_g4_scene: tree_kd against tree (R3: not claimed slower at 10000 and 100000; ratio > 1: kd slower)')
RES['scene'] = {}
for n in ('j100', '1240', '10000', '100000'):
    t = C('bp_g4_scene/tree/%s' % n)
    k = C('bp_g4_scene/tree_kd/%s' % n)
    c = L.cmp_(t, k)
    RES['scene'][n] = c
    v = 'not claimed'
    if claimed(c):
        v = ('STRONG ' if c['cl_r'] else '') + 'CLAIMED ' + ('SLOWER' if c['ratio'] > 1 else 'faster')
    P('- %-6s tree %s | tree_kd %s | kd/tree %s -> %s (w7 rule: %s)' % (
        n, L.fcell(t, 1, 2), L.fcell(k, 1, 2), L.fcmp(c), v, 'claimed' if w7(c) else 'not claimed'))
P('')
P('## Threshold check on tree (window_cmds: LO 144 and HI 152 in both families)')
fams = RES['families']
def lo_min(arm, rule):
    los = [fams[f][arm][rule]['LO'] for f in fams]
    return 'below the grid (< 128)' if any(x is None for x in los) else min(los)


for rule in ('ruling1', 'w7'):
    ok = all(fams[f]['tree'][rule]['LO'] == 144 and fams[f]['tree'][rule]['HI'] == 152 for f in fams)
    tb = lo_min('tree', rule)
    hi = max(fams[f]['tree'][rule]['HI'] for f in fams)
    tbk = lo_min('tree_kd', rule)
    hik = max(fams[f]['tree_kd'][rule]['HI'] for f in fams)
    P('- %s: tree -> TREE_BRUTE_MAX_ROWS %s, AUTO_TREE_LO/HI %s/%s: reproduces 144/152: %s; tree_kd -> %s, %s/%s' % (
        rule, tb, tb, hi, ok, tbk, tbk, hik))
for arm in ('tree', 'tree_kd'):
    if any(fams[f][arm]['loglog'] is None for f in fams):
        P('- L2 procedure beside (%s): crossovers %s; not computable (a family has no ratio <= 1 inside the grid)' % (
            arm, {f: fams[f][arm]['loglog'] for f in fams}))
        continue
    b = max(fams[f][arm]['loglog'] for f in fams)
    mag = 10 ** (math.floor(math.log10(b)) - 1)
    hi2 = round(b / mag) * mag
    P('- L2 procedure beside (%s): crossovers %s; the larger %.1f -> HI %d, LO %d' % (
        arm, {f: round(fams[f][arm]['loglog'], 1) for f in fams}, b, hi2, round(0.9 * hi2)))


def dump(o):
    if isinstance(o, dict):
        return {str(k): dump(v) for k, v in o.items()}
    if isinstance(o, (list, tuple)):
        return [dump(v) for v in o]
    return o


json.dump(dump(RES), open(os.path.join(L.HERE, 'g4.json'), 'w'), indent=1)
open(os.path.join(L.HERE, 'g4.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
