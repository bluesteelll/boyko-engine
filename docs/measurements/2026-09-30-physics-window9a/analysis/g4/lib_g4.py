"""Window 9a, group "C4-G4 thresholds + C4-G4-kd" (results-analyst, 2026-10-01). Read-only on raw/, rows9a.json,
bin/SHA256SUMS.

Statistics: ../c4ab/lib9a.py verbatim (cell, cmp_, ruling1, yn) = the window-8b synthlib = the window-8 lib8:
  cell = median over the used processes; i = IQR (inclusive quartiles)/median; s = 1.2533*SD/sqrt(K)/median;
  r = (max-min)/median; cmp_(A, B): ratio = B/A, flag x iff |B/A-1| > 2*hypot(x_A, x_B) and both K >= 3.
  Ruling 1: CLAIMED iff i AND s in every gating pass AND pooled, one sign; STRONG iff r also.

Per-process value (the window-8b G4 convention, win8b/analysis/f3/g4.py and f3.md "Method": "G4: the Flat mean of
times/iters over 10 samples, in us"): recomputed from CRITERION_HOME/<id>/new/sample.json. With SamplingMode::Flat
criterion has no slope estimate (estimates.json slope = null), so the printed point estimate IS this mean
(estimates.json mean.point_estimate); both are compared, neither is used.

Passes: only passes with a pass_done record of run 033740 (the resume). Slot rule (ruling 8): the original if clean and
valid, else the first clean valid re-run, else dropped (G4 and kd ran 0 re-runs).
"""
import json
import os
import re
import statistics
import sys
from collections import defaultdict

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'c4ab'))
import lib9a as L  # noqa: E402  statistics, clean_why, sha pins (the window-9a conventions)

RUN_TAG = '033740'
W9A = L.W9A
RAW = L.RAW
FAMS = ('uniform', 'disparity')
SIZES = (96, 104, 112, 120, 128, 136, 144, 152, 160)
KD_SIZES = (96, 112, 128)
# disparity_scene(n) = n small spheres + 4 giants (benches/support/bp_g4_scenes.rs:61-62), so its rows are n + 4
ROWS_EXTRA = {'uniform': 0, 'disparity': 4}
PROV = {'TREE_BRUTE_MAX_ROWS': 128, 'AUTO_TREE_LO': 128, 'AUTO_TREE_HI': 136}

TIME_RE = re.compile(r'^(bp_g4_\S+)\s*\n?\s*time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]', re.M)
KRE = re.compile(r'^bp_g4_(\w+)/(\w+): kernel (\w+) rows (\d+) pairs (\d+) tree members (\d+) static_rebuilds (\d+) '
                 r'wide (\d+) excluded (\d+) leaf_list_leaves (\d+) fallback_leaves (\d+) row_walk_leaves (\d+) '
                 r'kd_order_builds (\d+)', re.M)
UNIT_US = {'ns': 1e-3, 'us': 1.0, '\u00b5s': 1.0, 'ms': 1e3, 's': 1e6, 'ps': 1e-6}


def all_records():
    return L.all_records()


def closed_passes(recs):
    return {(r['block'], r['pass'], r['pass_attempt']) for r in recs
            if r.get('pass_done') and r.get('run_tag') == RUN_TAG}


def procs_of(block, recs):
    cp = closed_passes(recs)
    return [r for r in recs if r.get('block') == block and 'row' in r and r.get('attempt') != 'warmup'
            and r.get('run_tag') == RUN_TAG and (r['block'], r['pass'], r['pass_attempt']) in cp]


def unit_us(u):
    if u in UNIT_US:
        return UNIT_US[u]
    if u.endswith('s') and u[-2:-1] in ('µ', 'μ'):  # micro sign or Greek mu, however decoded
        return 1.0
    raise ValueError(repr(u))


def load_crit(r, sizes_receipt=SIZES):
    """Independent validity (not the driver flag) and the per-process values in us."""
    row = L.ROWS[r['row']]
    need = sorted(row['expect'])
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('hang'):
        why.append('hang')
    if not L.sha_ok(r):
        why.append('sha256')
    so = L.read(os.path.join(d, 'stdout.txt')).decode('utf-8', errors='strict')
    se = L.read(os.path.join(d, 'stderr.txt')).decode('utf-8', errors='strict')
    printed = defaultdict(list)
    for m in TIME_RE.finditer(so):
        printed[m.group(1)].append(float(m.group(4)) * unit_us(m.group(5)))
    vals, est_mean, n_iters = {}, {}, {}
    for i in need:
        sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
        ep = os.path.join(d, 'criterion', *i.split('/'), 'new', 'estimates.json')
        if not os.path.exists(sp):
            continue
        s = json.load(open(sp, encoding='utf-8'))
        if s.get('sampling_mode') != 'Flat' or len(s['times']) != 10 or len(s['iters']) != 10:
            why.append('%s sampling %s x%d' % (i, s.get('sampling_mode'), len(s['times'])))
        vals[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3  # ns -> us
        n_iters[i] = s['iters'][0]
        if os.path.exists(ep):
            e = json.load(open(ep, encoding='utf-8'))
            est_mean[i] = e['mean']['point_estimate'] / 1e3
            if e.get('slope') is not None:
                why.append('%s has a slope estimate (not Flat?)' % i)
    on_disk = set()
    for root, dirs, files in os.walk(os.path.join(d, 'criterion')):
        if os.path.basename(root) == 'new' and 'benchmark.json' in files:
            on_disk.add(json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8'))['full_id'])
    miss = [i for i in need if i not in vals or i not in printed or i not in on_disk]
    extra = sorted((set(printed) | on_disk) - set(need))
    dup = [i for i, v in printed.items() if len(v) != 1]
    if miss:
        why.append('missing %s' % miss[:3])
    if extra:
        why.append('extra %s' % extra[:3])
    if dup:
        why.append('dup %s' % dup[:3])
    w_pr = max((abs(printed[i][0] / vals[i] - 1) for i in vals if i in printed), default=None)
    w_est = max((abs(est_mean[i] / vals[i] - 1) for i in vals if i in est_mean), default=None)
    if w_pr is None or w_pr > 1e-4:
        why.append('printed vs sample.json %s' % w_pr)
    if w_est is None or w_est > 1e-9:
        why.append('estimates.json mean vs sample.json %s' % w_est)
    # the driver est_ms (its stdout parse) agrees with the printed values
    dr = (r.get('criterion') or {}).get('est_ms') or {}
    w_dr = max((abs(dr[i][1] * 1e3 / vals[i] - 1) for i in vals if i in dr), default=None)
    if w_dr is None or w_dr > 1e-4 or set(dr) != set(need):
        why.append('driver est_ms %s ids %d' % (w_dr, len(dr)))
    # kernel receipts printed before timing (every size of every family: the bench builds all scenes first)
    kr = defaultdict(dict)
    for fam, param, kern, rows_, pairs, mem, srb, wide, exc, ll, fb, rw, kdb in KRE.findall(se):
        kr[(fam, param)][kern] = dict(rows=int(rows_), pairs=int(pairs), members=int(mem), kdb=int(kdb),
                                      ll=int(ll), fb=int(fb), rw=int(rw))
    kbad = []
    for fam in FAMS:
        for n in sizes_receipt:
            v = kr.get((fam, str(n)))
            if not v or set(v) != {'LeafList', 'RowWalk', 'LeafListKd'}:
                kbad.append((fam, n, 'absent'))
                continue
            if len({x['pairs'] for x in v.values()}) != 1 or any(x['rows'] != n + ROWS_EXTRA[fam] for x in v.values()):
                kbad.append((fam, n, 'pairs/rows'))
            if v['LeafListKd']['kdb'] == 0 or v['LeafList']['kdb'] != 0 or v['RowWalk']['kdb'] != 0:
                kbad.append((fam, n, 'kd builds'))
            if any(x['members'] != 0 for x in v.values()):
                kbad.append((fam, n, 'members'))
    if kbad:
        why.append('kernel receipts %s' % kbad[:3])
    p = {'rec': r, 'block': r['block'], 'pass': r['pass'], 'round': r['round'], 'seq': r['seq'],
         'attempt': r['attempt'], 'rerun_no': r.get('rerun_no', 0), 'v': vals, 'est': est_mean, 'kr': dict(kr),
         'w_pr': w_pr, 'w_est': w_est, 'w_dr': w_dr, 'valid_why': why, 'clean_why': L.clean_why(r),
         'n_iters': n_iters, 'driver_valid': r.get('valid'), 'driver_contaminated': r.get('contaminated')}
    return p


def select(block, recs):
    procs = [load_crit(r) for r in procs_of(block, recs)]
    slots = defaultdict(list)
    for p in procs:
        slots[(p['block'], p['pass'], p['seq'])].append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items()):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p['rerun_no'])
        pick = next((c for c in orig + rer if not c['valid_why'] and not c['clean_why']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return procs, used, dropped


def by_pass(used, i):
    out = defaultdict(list)
    for p in used:
        out[p['pass']].append(p['v'][i])
    return dict(out)


def gid(fam, arm, n):
    return 'bp_g4_%s/%s/%d' % (fam, arm, n)


def verdict(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return ('STRONG ' if j['strong'] else '') + 'CLAIMED'


def fc(c, d=3):
    if c['K'] == 0:
        return 'n=0'
    return '%.*f us [%.*f-%.*f], IQR %.2f %%, SE %.2f %%, range %.2f %%, n=%d' % (
        d, c['median'], d, c['min'], d, c['max'], 100 * c['i'], 100 * c['s'], 100 * c['r'], c['K'])
