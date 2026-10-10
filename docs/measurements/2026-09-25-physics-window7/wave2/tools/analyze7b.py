"""Window 7 wave 2: the analyst's reduction, recomputed from raw/ with its own parsers.

It never imports tools/reduce7b.py and never reads the driver's per-process statistics ('mean_ms', 'cols',
'criterion', 'valid', 'contaminated'). From the record it takes only identity (block, pass, round, row, binary, W,
attempt, run tag, cwd, args, exit, exe sha256, mask) and the receipts (the 5-s/10-s CPU receipts and the
during-process witness), which exist nowhere else. Everything else is re-read from the files each process left:
  runner    stdout.txt SUMMARY line + run.csv (per step: wall_ns, manifolds, the span columns)
  Jolt      stdout.txt stat line + patch banner + per_frame_discrete_th<W>.csv
  criterion stdout.txt 'time:' lines, criterion/<id>/new/sample.json (Flat sampling: the point estimate is the
            mean of times/iters, recomputed here) and the kernel receipts in stderr.txt
Window 7's blocks A and B (P1A-jolt, P1B-jolt) are re-read the same way from window 7's own raw/ (the committed
record when this file sits in <window7>/wave2/tools/, else the scratch win7/ beside win7b/).

Statistic (windows 3-7): a cell is the median over K processes; r = min-max, i = IQR (linear quantiles), s =
1.2533 SD / sqrt K, each over the median. B against A is CLAIMED iff |B/A - 1| > 2 hypot(sA, sB) under BOTH r and s
(i printed); no claim when either side has K < 3. Against a constant bar b: CLAIMED iff |m/b - 1| > 2 r AND > 2 s.
Slot rule (windows 6/7): per (block, pass, round, row, binary, W) the original if valid and clean, else its re-run
if valid and clean, else the slot is dropped; clean = both receipts <= 5.0 % and no build process during it.

Writes analyst/tables.txt and analyst/reduction.json beside tools/.
"""
import csv
import json
import math
import os
import re
import statistics
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
W2 = os.path.dirname(HERE)
RAW = os.path.join(W2, 'raw')
GATE = os.path.join(W2, 'gate')
OUTD = os.path.join(W2, 'analyst')


def _find_w7():
    for c in (os.path.dirname(W2), os.path.join(os.path.dirname(W2), 'win7')):
        if os.path.exists(os.path.join(c, 'rows7.json')) and os.path.exists(os.path.join(c, 'raw', 'runs.jsonl')):
            return c
    raise SystemExit('window 7 record not found beside ' + W2)


W7 = _find_w7()
RAW7 = os.path.join(W7, 'raw')
GATE7 = os.path.join(W7, 'gate')
BUSY = 5.0
JOLT_HASH = '0xb8522b4e3fc62cfe'
WIN3_JOLT56 = {1: 9.8281962, 2: 5.7704603, 4: 3.5814395, 8: 2.5692559, 16: 2.3884695}
WIN6_TQ = {'P3 (plain layout, 648 chars)': 0.2102, 'P5g (padded layout, 654 chars)': 0.2354}
WIN4_G4 = {'uniform': {64: 0.619, 128: 1.137, 256: 1.997}, 'disparity': {64: 0.500, 128: 0.946, 256: 1.543}}
C2_BAR, ATTR_A, BAND, ROWS_Q = 0.235, 0.21, 0.186, 1240
SPAN_COLS = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')

OUT = []
RES = {'checks': {}, 'cells': {}, 'cmp': {}, 'decisions': {}, 'outliers': {}, 'witness': {}}


def P(s=''):
    OUT.append(s)
    print(s)


# ----------------------------------------------------------------------------------------------- statistics
def quantile(xs, q):
    """Linear interpolation between order statistics (type 7 = 'inclusive')."""
    xs = sorted(xs)
    h = (len(xs) - 1) * q
    lo = math.floor(h)
    return xs[lo] + (h - lo) * (xs[min(lo + 1, len(xs) - 1)] - xs[lo])


def cell(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    k = len(xs)
    m = statistics.median(xs)
    sd = statistics.stdev(xs) if k >= 2 else 0.0
    iq = quantile(xs, 0.75) - quantile(xs, 0.25) if k >= 2 else 0.0
    se = 1.2533 * sd / math.sqrt(k)
    return {'K': k, 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / m, 'i': iq / m, 's': se / m,
            'values': xs}


def cmp_(a, b):
    """B against A."""
    if not a or not b:
        return None
    ratio = b['median'] / a['median']
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    ok = a['K'] >= 3 and b['K'] >= 3
    cl = {k: ok and abs(ratio - 1) > bars[k] for k in 'ris'}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'],
            'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def vs_bar(c, bar):
    if not c:
        return None
    e = c['median'] / bar - 1
    ok = c['K'] >= 3
    cr, cs = ok and abs(e) > 2 * c['r'], ok and abs(e) > 2 * c['s']
    return {'median': c['median'], 'bar': bar, 'effect': e, 'side': 'above' if e > 0 else 'below', 'cl_r': cr,
            'cl_s': cs, 'claimed': cr and cs, 'bar_r': 2 * c['r'], 'bar_s': 2 * c['s']}


def yn(c):
    return '/'.join('Y' if c[f'cl_{k}'] else 'n' for k in 'ris')


def fc(c, d=4):
    if not c:
        return 'n/a'
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] K={c['K']} "
            f"({100 * c['r']:.2f}/{100 * c['i']:.2f}/{100 * c['s']:.2f} %)")


def fcmp(c):
    if not c:
        return 'n/a'
    return (f"{c['ratio']:.4f} ({100 * (c['ratio'] - 1):+.2f} %, {c['delta']:+.4f}); bars r/i/s "
            f"{100 * c['bar_r']:.2f}/{100 * c['bar_i']:.2f}/{100 * c['bar_s']:.2f} %; {yn(c)}"
            f"{' CLAIMED' if c['claimed'] else ' not claimed'}")


def fbar(v):
    return (f"{v['median']:.4f} vs {v['bar']} ({100 * v['effect']:+.2f} %; 2r {100 * v['bar_r']:.2f} %, 2s "
            f"{100 * v['bar_s']:.2f} %; r/s {'Y' if v['cl_r'] else 'n'}/{'Y' if v['cl_s'] else 'n'}) -> "
            f"{v['side'] + ' CLAIMED' if v['claimed'] else 'on the bar'}")


# ----------------------------------------------------------------------------------------------- inputs
def read_sums(path):
    out = {}
    for line in open(path, encoding='utf-8'):
        if line.strip():
            h, p = line.split(None, 1)
            out[p.strip().lstrip('*').replace('\\', '/')] = h
    return out


class Window:
    """One window's record: its rows, binaries, fixtures and SHA256SUMS."""

    def __init__(self, root, rows_name):
        self.root = root
        self.raw = os.path.join(root, 'raw')
        rows = json.load(open(os.path.join(root, rows_name), encoding='utf-8'))
        self.rowdef = {r['id']: r for r in rows['rows']}
        self.bins = rows['binaries']
        self.protocol = rows['protocol']
        self.sums = read_sums(os.path.join(root, 'bin', 'SHA256SUMS'))

    def bin_sha(self, key):
        exe = self.bins[key]['exe'].replace('\\', '/')
        for p, h in self.sums.items():
            if p == exe or p.endswith(exe) or exe.endswith(p):
                return h
        return None

    def local_dir(self, rec):
        parts = rec['cwd'].replace('\\', '/').split('/')
        return os.path.join(self.raw, parts[-2], parts[-1])


def summary_of(d):
    p = os.path.join(d, 'stdout.txt')
    if not os.path.exists(p):
        return None
    for line in open(p, encoding='utf-8', errors='replace'):
        if line.startswith('SUMMARY '):
            return json.loads(line[len('SUMMARY '):])
    return None


def read_run_csv(d):
    p = os.path.join(d, 'run.csv')
    if not os.path.exists(p):
        return None
    cols = defaultdict(dict)
    n = 0
    with open(p, encoding='utf-8') as f:
        rd = csv.reader(f)
        head = next(rd)
        for row in rd:
            if not row:
                continue
            n += 1
            s = int(row[0])
            for h, v in zip(head[1:], row[1:]):
                if v != '':
                    cols[h][s] = float(v)
    return n, cols


def win_stat(series, a, b, stat):
    xs = [v for s, v in series.items() if a <= s < b]
    if not xs:
        return None
    return statistics.fmean(xs) if stat == 'mean' else statistics.median(xs)


FIXTURE_HASH = {}


def load_fixtures():
    for path in (os.path.join(GATE, 'fixtures', 'fixtures.json'), os.path.join(GATE7, 'fixtures', 'fixtures.json')):
        for k, v in json.load(open(path, encoding='utf-8')).items():
            h = v.get('hash')
            if k in FIXTURE_HASH and FIXTURE_HASH[k] != h:
                raise SystemExit(f'fixture {k} disagrees between windows: {FIXTURE_HASH[k]} vs {h}')
            FIXTURE_HASH[k] = h


UNIT_MS = {'ps': 1e-9, 'ns': 1e-6, 'µs': 1e-3, 'us': 1e-3, 'ms': 1.0, 's': 1e3}
TIME_RE = re.compile(r'(bp_g4_[A-Za-z_]+/[A-Za-z_]+/\d+)\s+time:\s+\[([\d.]+)\s*(\S+)\s+([\d.]+)\s*(\S+)\s+'
                     r'([\d.]+)\s*(\S+)\]')
KERNEL_RE = re.compile(r'^bp_g4_(\w+)/(\d+): kernel (\w+) rows (\d+) pairs (\d+) .*leaf_list_leaves (\d+) '
                       r'fallback_leaves (\d+) row_walk_leaves (\d+)', re.M)


def expected_criterion_ids(filt):
    """Expand the Q3 filter ^bp_g4_(f1|f2)/((a|b)/(n1|...)|c/(m1|...))$ into its ids."""
    m = re.fullmatch(r'\^bp_g4_\(([^)]*)\)/\(\(([^)]*)\)/\(([^)]*)\)\|(\w+)/\(([^)]*)\)\)\$', filt)
    fams, arms, sizes, arm2, sizes2 = m.groups()
    ids = []
    for f in fams.split('|'):
        ids += [f'bp_g4_{f}/{a}/{n}' for a in arms.split('|') for n in sizes.split('|')]
        ids += [f'bp_g4_{f}/{arm2}/{n}' for n in sizes2.split('|')]
    return ids


def recompute(win, rec):
    """(mine, problems): every statistic this analysis uses, re-read from the process's own files."""
    row = win.rowdef[rec['row']]
    d = win.local_dir(rec)
    W = rec['W']
    probs, mine = [], {'dir': d, 'cols': {}}
    if rec.get('exit') != 0:
        probs.append(f"exit {rec.get('exit')}")
    sha = win.bin_sha(rec['binary'])
    if sha is None or rec.get('exe_sha256') != sha:
        probs.append('exe sha256 != SHA256SUMS')
    if rec.get('mask_readback') != '0xffff':
        probs.append(f"mask {rec.get('mask_readback')}")
    mine['cmdline_chars'] = sum(len(x) + 1 for x in rec['args'])
    mine['cwd_len'] = len(rec['cwd'])
    kind = rec['kind']
    if kind == 'runner':
        sm = summary_of(d)
        rc = read_run_csv(d)
        if not sm or not rc:
            return mine, probs + ['no SUMMARY or run.csv']
        n, cols = rc
        mine['summary'] = sm
        if n != row['steps']:
            probs.append(f'{n} csv rows != {row["steps"]}')
        if sm.get('void_steps') != 0:
            probs.append(f"void_steps {sm.get('void_steps')}")
        if sm.get('expect_pose') != 'match':
            probs.append(f"expect_pose {sm.get('expect_pose')}")
        if sm.get('pose_hash') != FIXTURE_HASH.get(row['pose_ref']):
            probs.append(f"pose {sm.get('pose_hash')} != {row['pose_ref']}")
        if sm.get('workers') != W or (sm.get('threads') or {}).get('pool_workers') not in (None, W):
            probs.append('workers != W')
        if sm.get('target_env') != 'msvc':
            probs.append('target_env')
        if bool(sm.get('armed')) != bool(row['armed']):
            probs.append('armed flag')
        if row['armed'] and sm.get('drops_total') != 0:
            probs.append(f"drops {sm.get('drops_total')}")
        if sm.get('disarmed_ring_traffic') not in (None, 0):
            probs.append('disarmed ring traffic')
        cfg = sm.get('config') or {}
        if cfg.get('broadphase') != row['broadphase']:
            probs.append(f"broadphase {cfg.get('broadphase')}")
        td = sm.get('broadphase_tree') or {}
        if row['broadphase'] == 'Tree':
            if (td.get('static_rebuilds'), td.get('members'), td.get('evictions')) != (1, 1, 0):
                probs.append(f'TreeDiag {td}')
        elif any(v for v in td.values()):
            probs.append('TreeDiag on AllPairs')
        args = row['args']
        if '--contact-reuse' in args:
            if cfg.get('contact_reuse') != (args[args.index('--contact-reuse') + 1] == 'on'):
                probs.append('contact_reuse config')
        elif cfg.get('contact_reuse'):
            probs.append('contact reuse on without the flag')
        if row.get('canary_of'):
            if not sm.get('canary_ns'):
                probs.append('canary_ns missing')
        elif sm.get('canary_ns'):
            probs.append('canary_ns on a non-canary row')
        a, b = row['window']
        wall = cols.get('wall_ns', {})
        mine['mean_ms'] = win_stat(wall, a, b, 'mean') / 1e6
        # the SUMMARY's own window mean must agree with the csv (the runner and the csv are two writers)
        if abs(sm['window_mean_ns'] / 1e6 - mine['mean_ms']) > 1e-6 * mine['mean_ms']:
            probs.append('SUMMARY window_mean_ns != csv mean')
        for (x, y) in [(a, b)] + [tuple(w) for w in (row.get('metric_windows') or [])]:
            key = f'{x}..{y}'
            mine['cols'][key] = {}
            for c, series in cols.items():
                mm = win_stat(series, x, y, 'mean')
                if mm is not None:
                    mine['cols'][key][c] = {'mean': mm, 'median': win_stat(series, x, y, 'median')}
        mine['canary_ms'] = (sm.get('canary_ns') or 0) / 1e6 or None
    elif kind == 'jolt':
        p = os.path.join(d, f'per_frame_discrete_th{W}.csv')
        txt = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
        stats = re.findall(r'^\s*Discrete,\s*(\d+),\s*([\d.]+),\s*(0x[0-9a-f]+)\s*$', txt, re.M)
        banner = [l for l in txt.splitlines() if 'boyko-parity-patch' in l]
        if not os.path.exists(p):
            return mine, probs + ['no per_frame csv']
        with open(p, encoding='utf-8') as f:
            rd = csv.reader(f)
            next(rd)
            fr = {int(x[0]): float(x[1]) for x in rd if x}
        if len(fr) != 500:
            probs.append(f'{len(fr)} frames')
        if len(stats) != 1:
            probs.append(f'{len(stats)} stat lines')
        elif int(stats[0][0]) != W or stats[0][2] != JOLT_HASH:
            probs.append(f'stat line {stats[0]}')
        if len(banner) != 1 or 'boyko-parity-patch v1' not in banner[0] or 'allow_sleep=0, receipt=0' not in banner[0]:
            probs.append('patch banner')
        a, b = row['window']
        mine['mean_ms'] = win_stat(fr, a, b, 'mean')
        for (x, y) in [(a, b)] + [tuple(w) for w in (row.get('metric_windows') or [])]:
            mine['cols'][f'{x}..{y}'] = {'wall_ns': {'mean': win_stat(fr, x, y, 'mean') * 1e6,
                                                     'median': win_stat(fr, x, y, 'median') * 1e6}}
    elif kind == 'criterion':
        so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
        se = open(os.path.join(d, 'stderr.txt'), encoding='utf-8', errors='replace').read()
        need = expected_criterion_ids(row['args'][-1])
        printed = {}
        for m in TIME_RE.finditer(so):
            printed.setdefault(m.group(1), []).append(float(m.group(4)) * UNIT_MS[m.group(5)])
        est = {}
        for i in need:
            sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
            if not os.path.exists(sp):
                continue
            s = json.load(open(sp, encoding='utf-8'))
            if s.get('sampling_mode') != 'Flat' or len(s['times']) != 10:
                probs.append(f'{i}: sampling {s.get("sampling_mode")} x{len(s["times"])}')
            est[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e6  # ns -> ms
        miss = [i for i in need if i not in est or i not in printed]
        extra = sorted(set(printed) - set(need))
        if miss:
            probs.append(f'{len(miss)} ids without an estimate: {miss[:3]}')
        if extra:
            probs.append(f'{len(extra)} unexpected ids: {extra[:3]}')
        dup = [i for i, v in printed.items() if len(v) != 1]
        if dup:
            probs.append(f'ids printed twice: {dup[:3]}')
        # the printed estimate is 5 significant figures of the recomputed mean
        worst = max((abs(printed[i][0] / est[i] - 1) for i in est if i in printed), default=0.0)
        if worst > 1e-4:
            probs.append(f'printed estimate differs from sample.json mean by {worst:.2e}')
        mine['criterion_worst_rel'] = worst
        # the bench's own oracle: each timed size printed a LeafList and a RowWalk receipt with equal pair counts
        kr = defaultdict(dict)
        for fam, n, kern, rows_, pairs, ll, fb, rw in KERNEL_RE.findall(se):
            kr[(fam, int(n))][kern] = (int(pairs), int(ll), int(fb), int(rw))
        sizes = sorted({int(i.rsplit('/', 1)[1]) for i in need})
        for fam in ('uniform', 'disparity'):
            for n in sizes:
                k = kr.get((fam, n), {})
                if set(k) != {'LeafList', 'RowWalk'} or k['LeafList'][0] != k['RowWalk'][0]:
                    probs.append(f'kernel receipt {fam}/{n}: {k}')
                elif k['LeafList'][2] or k['RowWalk'][1] or k['LeafList'][1] == 0 or k['RowWalk'][3] == 0:
                    probs.append(f'kernel leaves {fam}/{n}: {k}')
        mine['kernel_receipts'] = len(kr)
        mine['est_ms'] = est
    return mine, probs


def select(win, recs, label, blocks=None):
    """Recompute every non-warm-up process (of `blocks`, when given) and apply the slot rule."""
    recs = [r for r in recs if blocks is None or r.get('block') in blocks]
    procs = [r for r in recs if 'row' in r]
    markers = [r for r in recs if 'row' not in r]
    done_tag = {(m['block'], m['pass']): m['run_tag'] for m in markers if m.get('pass_done')}
    voided = [r for r in recs if r.get('voided_pass')]
    att = defaultdict(int)
    for r in procs:
        att[r['attempt']] += 1
    probs_all, mism = [], defaultdict(int)
    for r in procs:
        if r['attempt'] == 'warmup':
            continue
        mine, probs = recompute(win, r)
        r['_mine'], r['_probs'] = mine, probs
        rb, ra = r['receipt_before']['cpu_avg'], r['receipt_after']['cpu_avg']
        r['_valid'] = not probs
        r['_clean'] = rb <= BUSY and ra <= BUSY and not r.get('build_proc_during')
        if probs:
            probs_all.append((r['block'], r['pass'], r['seq'], r['attempt'], r['row'], r['binary'], r['W'], probs))
        # the driver's flags are compared, never used
        if r['_valid'] != bool(r.get('valid')):
            mism['valid flag'] += 1
        if r['_clean'] != (not r.get('contaminated')):
            mism['clean flag'] += 1
        if mine.get('mean_ms') is not None and r.get('mean_ms') is not None and r['kind'] != 'criterion':
            if abs(mine['mean_ms'] - r['mean_ms']) > 1e-9 * max(1.0, r['mean_ms']):
                mism['mean_ms'] += 1
        for w_, cs in (r.get('cols') or {}).items():
            for c, v in cs.items():
                m2 = (mine['cols'].get(w_) or {}).get(c)
                if m2 is None:
                    mism['col missing'] += 1
                    continue
                for st in ('mean', 'median'):
                    if st in v and abs(m2[st] - v[st]) > 1e-9 * max(1.0, abs(v[st])):
                        mism[f'col {st}'] += 1
        if r['kind'] == 'criterion':
            for i, v in ((r.get('criterion') or {}).get('est_ms') or {}).items():
                if i in mine.get('est_ms', {}) and abs(v[1] / mine['est_ms'][i] - 1) > 1e-4:
                    mism['criterion est'] += 1
    slots = defaultdict(dict)
    for r in procs:
        if r['attempt'] in ('original', 'rerun') and r['run_tag'] == done_tag.get((r['block'], r['pass'])):
            slots[(r['block'], r['pass'], r['round'], r['row'], r['binary'], r['W'])][r['attempt']] = r
    chosen, dropped, by_rerun = [], [], 0
    for k, a in slots.items():
        pick = next((a[t] for t in ('original', 'rerun') if t in a and a[t]['_valid'] and a[t]['_clean']), None)
        if pick:
            chosen.append(pick)
            by_rerun += pick['attempt'] == 'rerun'
        else:
            dropped.append(k)
    P(f'- {label}: {len(recs)} records = {len(procs)} processes {dict(att)} + {len(markers)} pass markers '
      f'({len(done_tag)} passes done); voided pass records {len(voided)}')
    P(f'  - validity problems in {len(probs_all)} non-warm-up processes'
      + (': ' + '; '.join(map(str, probs_all[:6])) if probs_all else ''))
    P(f"  - against the driver's per-process fields: "
      + (str(dict(mism)) if mism else 'every valid flag, clean flag, mean_ms, cols value and criterion estimate reproduces'))
    P(f'  - slots {len(slots)}: used {len(chosen)} ({by_rerun} by their re-run), dropped {len(dropped)}'
      + (': ' + ', '.join(f'{k[0]} p{k[1]} r{k[2]} {k[3]} {k[4]} W{k[5]}' for k in sorted(dropped)) if dropped else ''))
    RES['checks'][label] = {'records': len(recs), 'processes': len(procs), 'attempts': dict(att),
                            'passes_done': len(done_tag), 'voided': len(voided), 'problems': probs_all,
                            'driver_mismatch': dict(mism), 'slots': len(slots), 'used': len(chosen),
                            'used_by_rerun': by_rerun, 'dropped': [list(k) for k in sorted(dropped)]}
    return chosen


def pid(r):
    return f"{r['block']}-p{r['pass']} #{r['seq']:03d} r{r['round']} {r['row']} {r['binary']} W{r['W']}" + \
        (' (re-run)' if r['attempt'] == 'rerun' else '')


def receipt_txt(r):
    def top(lst):
        return f"{lst[0]['name']} {lst[0]['cpu_s']:.2f} s" if lst else '-'
    return (f"start {r['start'][11:19]}; receipts before/after {r['receipt_before']['cpu_avg']:.2f} / "
            f"{r['receipt_after']['cpu_avg']:.2f} % (top {top(r['receipt_before']['top5'])} / "
            f"{top(r['receipt_after']['top5'])}); during-process witness {r['others_busy_pct']:.2f} % "
            f"(top {top(r['others_top5'])})")


def main():
    try:
        sys.stdout.reconfigure(encoding='utf-8', newline='\n')
    except (AttributeError, ValueError):
        pass
    os.makedirs(OUTD, exist_ok=True)
    load_fixtures()
    w2 = Window(W2, 'rows7b.json')
    w7 = Window(W7, 'rows7.json')
    recs2 = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
    recs7 = [json.loads(l) for l in open(os.path.join(RAW7, 'runs.jsonl'), encoding='utf-8') if l.strip()]
    P('# Window 7 wave 2: the analyst\'s reduction (tools/analyze7b.py), recomputed from raw/')
    P(f'inputs: wave 2 {RAW}; window 7 (blocks A, B) {RAW7}')
    P('\n## 0. Selection and input checks')
    ch2 = select(w2, recs2, 'wave 2')
    ch7 = select(w7, recs7, 'window 7, blocks P1A-jolt and P1B-jolt (= Q1 blocks A and B)', ('P1A-jolt', 'P1B-jolt'))
    BN = {'P1A-jolt': 'A', 'P1B-jolt': 'B', 'Q1C': 'C', 'Q1D': 'D'}
    for r in ch7 + ch2:
        r['_blk'] = BN.get(r['block'], r['block'])
    pool = ch2 + ch7

    # launch-context lengths (Q1 and Q2 rows pin them)
    miss = []
    for r in ch2:
        row = w2.rowdef[r['row']]
        tgt = (row.get('cwd_len') or {}).get(str(r['W']))
        ref = (row.get('cmdline_ref') or {}).get(str(r['W']))
        if tgt is not None and (r['_mine']['cwd_len'] != tgt or r['_mine']['cmdline_chars'] != ref):
            miss.append((pid(r), r['_mine']['cwd_len'], tgt, r['_mine']['cmdline_chars'], ref))
    crit = [r for r in ch2 if r['kind'] == 'criterion']
    P(f"- criterion (Q3): {len(crit)} processes x {[len(r['_mine']['est_ms']) for r in crit]} ids recomputed from "
      f"sample.json (Flat, 10 samples each); the printed estimate matches the recomputed mean to "
      f"{max(r['_mine']['criterion_worst_rel'] for r in crit):.1e} relative at worst; kernel receipts (LeafList and "
      f"RowWalk, equal pair counts) for {[r['_mine']['kernel_receipts'] for r in crit]} family-size pairs, every "
      f"timed size included")
    P(f'- launch-context lengths (Q1, Q2 rows, used set): {len(miss)} misses against the pinned cwd length and '
      f'command-line characters' + (f': {miss[:4]}' if miss else ''))
    RES['checks']['length_misses'] = miss

    # receipts
    rcp = {}
    for r in recs2:
        if 'row' in r:
            for rc in (r['receipt_before'], r['receipt_after']):
                rcp[rc['time']] = rc
    vals = sorted(rc['cpu_avg'] for rc in rcp.values())
    over = [rc for rc in rcp.values() if rc['cpu_avg'] > BUSY]
    pres = sum(1 for r in recs2 if 'row' in r for rc in (r['receipt_before'], r['receipt_after'])
               if (rc.get('presence') or {}).get('build') or (rc.get('presence') or {}).get('lane'))
    pres += sum(1 for r in recs2 if 'row' in r and (r.get('build_proc_during') or r.get('void_names_new_during')))
    P(f'- wave-2 receipts (distinct, warm-ups included): {len(vals)}; median {statistics.median(vals):.2f} %, p90 '
      f'{vals[int(0.9 * len(vals))]:.2f} %, max {vals[-1]:.2f} %, {len(over)} over 5 % '
      f'({[(rc["time"][11:19], rc["cpu_avg"], rc["top5"][0]["name"]) for rc in over]}); build/lane presence at a '
      f'receipt or during a process: {pres}')
    RES['checks']['receipts'] = {'n': len(vals), 'median': statistics.median(vals), 'p90': vals[int(0.9 * len(vals))],
                                 'max': vals[-1], 'over5': len(over)}
    reruns = [r for r in recs2 if r.get('attempt') == 'rerun']
    for r in reruns:
        orig = next(x for x in recs2 if x.get('attempt') == 'original' and x['block'] == r['block']
                    and x['pass'] == r['pass'] and x['round'] == r['round'] and x['row'] == r['row']
                    and x['binary'] == r['binary'] and x['W'] == r['W'])
        P(f"  - re-run: {pid(r)}; the original's {receipt_txt(orig)}; the re-run's {receipt_txt(r)}")

    # the during-process witness by pass, used set, Q1's four blocks and every wave-2 pass
    P('- the during-process witness (others_busy_pct) and the 5-s receipts, used set, by pass '
      '(median; p90 / max of the witness)')
    byp = defaultdict(list)
    for r in pool:
        byp[(r['block'], r['pass'])].append(r)
    order = ['P1A-jolt', 'P1B-jolt', 'Q1C', 'Q2C', 'Q3', 'Q4', 'Q2D', 'Q1D']
    for blk in order:
        for ps in (0, 1):
            rs = byp.get((blk, ps))
            if not rs:
                continue
            wv = sorted(r['others_busy_pct'] for r in rs)
            rv = sorted(x for r in rs for x in (r['receipt_before']['cpu_avg'], r['receipt_after']['cpu_avg']))
            tops = defaultdict(int)
            for r in rs:
                if r['others_top5']:
                    tops[r['others_top5'][0]['name']] += 1
            ts = sorted(r['start'][11:19] for r in rs)
            t2 = sorted(tops.items(), key=lambda kv: -kv[1])[:2]
            P(f'  - {blk}-p{ps} ({ts[0]}-{ts[-1]}, n={len(rs)}): witness {statistics.median(wv):.2f} % '
              f'({wv[int(0.9 * len(wv))]:.2f} / {wv[-1]:.2f}); receipts {statistics.median(rv):.2f} %; top other {t2}')
            RES['witness'][f'{blk}-p{ps}'] = {'n': len(rs), 'witness_median': statistics.median(wv),
                                              'witness_max': wv[-1], 'receipt_median': statistics.median(rv),
                                              'from': ts[0], 'to': ts[-1], 'top': t2}

    def C(row, key, w, fn, blocks, excl_miss=True):
        rs = [r for r in pool if r['row'] == row and r['binary'] == key and r['W'] == w and r['_blk'] in blocks]
        return cell([fn(r) for r in rs])

    def procs_of(row, key, w, blocks):
        return [r for r in pool if r['row'] == row and r['binary'] == key and r['W'] == w and r['_blk'] in blocks]

    def wall(r):
        return r['_mine']['mean_ms']

    def zone(win, col, stat='mean', scale=1e-6):
        def f(r):
            v = (r['_mine']['cols'].get(win) or {}).get(col)
            return v[stat] * scale if v else None
        return f

    def keep(name, c):
        RES['cells'][name] = c
        return c

    # ================================================================================================== Q1
    P('\n## 1. Q1: the W8 headline over four separated blocks (trunk 93b2615b default tree row vs Jolt v5.6.0)')
    P('A = window 7 P1A-jolt, B = window 7 P1B-jolt, C = Q1C (first block of this run), D = Q1D (last). Statistic: '
      'the process mean over [0,500). Rule (plan.md 4, Q1): HOLDS iff claimed in A, B, C, D AND pooled over A-D, '
      'same direction.')
    BL = {'A': ('A',), 'B': ('B',), 'C': ('C',), 'D': ('D',), 'pooled A-D': ('A', 'B', 'C', 'D'),
          'pooled C+D': ('C', 'D')}
    # Jolt's manifold count: re-read from window 7's -receipt CSVs
    mj = {}
    for w in (1, 8):
        p = os.path.join(GATE7, 'jolt_receipt', f'j56_W{w}', f'receipt_discrete_th{w}.csv')
        with open(p, encoding='utf-8') as f:
            rd = csv.reader(f)
            next(rd)
            mf = {int(x[0]): float(x[1]) for x in rd if x}
        mj[w] = statistics.fmean(v for k, v in mf.items() if 100 <= k < 500)
    P(f'- Jolt manifolds per frame over [100,500), re-read from window 7 gate/jolt_receipt: W1 {mj[1]:.4f}, W8 '
      f'{mj[8]:.4f} (the count does not depend on the thread count; W4/W16 use it)')
    MJ = mj[8]
    q1 = {}
    for w in (4, 8, 16):
        P(f'\n### W={w}')
        cw = {}
        for row, key in (('H-trk-tree', 'trk'), ('H-trk-ap', 'trk'), ('H-jolt56', 'j56')):
            for bn, bl in BL.items():
                cw[(row, bn)] = keep(f'{row}@W{w} [{bn}] [0,500) ms', C(row, key, w, wall, bl))
                for sub in ('0..100', '100..500'):
                    cw[(row, bn, sub)] = C(row, key, w, zone(sub, 'wall_ns'), bl)
                if row != 'H-jolt56':
                    cw[(row, bn, 'mf')] = C(row, key, w, zone('100..500', 'manifolds', scale=1.0), bl)
            P(f'- {row}: ' + '; '.join(f'{bn} {fc(cw[(row, bn)])}' for bn in BL))
        for ours in ('H-trk-tree', 'H-trk-ap'):
            res = {}
            for bn in BL:
                c = cmp_(cw[('H-jolt56', bn)], cw[(ours, bn)])
                res[bn] = c
                RES['cmp'][f'Q1 {ours}/jolt56 W{w} [{bn}] [0,500)'] = c
            for sub in ('0..100', '100..500'):
                RES['cmp'][f'Q1 {ours}/jolt56 W{w} [pooled A-D] [{sub})'] = cmp_(
                    cw[('H-jolt56', 'pooled A-D', sub)], cw[(ours, 'pooled A-D', sub)])
            P(f'- **{ours} / Jolt W{w} [0,500)**:')
            for bn in BL:
                P(f'  - {bn}: {fcmp(res[bn])}')
            need = [res[b] for b in ('A', 'B', 'C', 'D', 'pooled A-D')]
            holds = all(x['claimed'] for x in need) and len({x['ratio'] > 1 for x in need}) == 1
            flags = ', '.join(f'{b} {"Y" if res[b]["claimed"] else "n"}' for b in ('A', 'B', 'C', 'D', 'pooled A-D'))
            verdict = ('HOLDS' if holds else 'does NOT hold') + f' ({flags})'
            q1[(ours, w)] = {'verdict': verdict, 'ratios': {b: res[b]['ratio'] for b in BL},
                             'flags': {b: yn(res[b]) for b in BL}}
            P(f'  - rule: {verdict}')
            for sub in ('100..500',):
                P(f'  - [{sub}) per block: ' + '; '.join(
                    f"{bn} {fcmp(cmp_(cw[('H-jolt56', bn, sub)], cw[(ours, bn, sub)]))}" for bn in BL))
            # per manifold, each side's own count
            pm = {}
            for bn in BL:
                to, tj, mo = cw[(ours, bn, '100..500')], cw[('H-jolt56', bn, '100..500')], cw[(ours, bn, 'mf')]
                if to and tj and mo:
                    uo, uj = to['median'] * 1e6 / mo['median'], tj['median'] * 1e6 / MJ
                    pm[bn] = (uo, uj, uo / uj, mo['median'])
            q1[(ours, w)]['per_manifold'] = pm
            P('  - per manifold [100,500), ns (ours over our own count, Jolt over 8,489.0): ' + '; '.join(
                f'{bn} {v[0]:.1f} / {v[1]:.1f} = {v[2]:.3f}x' for bn, v in pm.items())
              + f" (our count {', '.join(sorted({f'{v[3]:.4f}' for v in pm.values()}))})")
        for row in ('H-jolt56', 'H-trk-tree', 'H-trk-ap'):
            parts = []
            for a, b in (('A', 'B'), ('A', 'C'), ('C', 'D'), ('B', 'D')):
                c = cmp_(cw[(row, a)], cw[(row, b)])
                RES['cmp'][f'Q1 block term {row} W{w} {b} vs {a}'] = c
                parts.append(f'{b}/{a} {c["ratio"]:.4f} {yn(c)}')
            P(f'- block terms {row}: ' + '; '.join(parts))
        P(f"- window term (context): Jolt / window 3 = " + ', '.join(
            f"{bn} {cw[('H-jolt56', bn)]['median'] / WIN3_JOLT56[w]:.4f}" for bn in ('A', 'B', 'C', 'D', 'pooled A-D')))

        # the processes that set each W8 cell's range (and the W16 cells', for the near-parity reading)
        if w in (8, 16):
            P(f'- **the processes at the ends of each W{w} cell** (value ms; the rest of the cell; the range without it)')
            out = []
            for row, key in (('H-trk-tree', 'trk'), ('H-jolt56', 'j56')):
                for bn in ('A', 'B', 'C', 'D'):
                    rs = sorted(procs_of(row, key, w, (bn,)), key=wall)
                    if len(rs) < 3:
                        continue
                    xs = [wall(r) for r in rs]
                    m = statistics.median(xs)
                    hi, lo = rs[-1], rs[0]
                    gap_hi, gap_lo = xs[-1] - xs[-2], xs[1] - xs[0]
                    far = hi if gap_hi >= gap_lo else lo
                    rest = [wall(r) for r in rs if r is not far]
                    rng_wo = (max(rest) - min(rest)) / statistics.median(rest)
                    rec = {'row': row, 'block': bn, 'process': pid(far), 'value': wall(far), 'median': m,
                           'r': (xs[-1] - xs[0]) / m, 'r_without': rng_wo, 'others': [round(x, 4) for x in xs],
                           'receipt_before': far['receipt_before']['cpu_avg'],
                           'receipt_after': far['receipt_after']['cpu_avg'], 'witness': far['others_busy_pct'],
                           'witness_top': far['others_top5'][:2], 'start': far['start']}
                    out.append(rec)
                    P(f"  - {row} [{bn}] median {m:.4f}, r {100 * rec['r']:.2f} % -> {100 * rng_wo:.2f} % without "
                      f"{pid(far)} = {wall(far):.4f} ms ({100 * (wall(far) / m - 1):+.2f} % of the median); "
                      f"{receipt_txt(far)}; the cell: {[round(x, 4) for x in xs]}")
            RES['outliers'][f'W{w}'] = out
    RES['decisions']['Q1'] = {f'{k[0]} W{k[1]}': v for k, v in q1.items()}

    # the rule's cells at W8 re-read with the single range-setting process removed (DIAGNOSTIC: not the protocol)
    P('\n- **diagnostic, not the protocol:** the W8 default row against Jolt with each block\'s single range-setting '
      'process removed (the process named above; the protocol cell is the one in the table):')
    dg = {}
    for bn in ('A', 'B', 'C', 'D'):
        o = procs_of('H-trk-tree', 'trk', 8, (bn,))
        j = procs_of('H-jolt56', 'j56', 8, (bn,))
        drop = {x['process'] for x in RES['outliers']['W8'] if x['block'] == bn}
        o2 = cell([wall(r) for r in o if pid(r) not in drop])
        j2 = cell([wall(r) for r in j if pid(r) not in drop])
        c = cmp_(j2, o2)
        dg[bn] = c
        P(f'  - {bn}: ours {fc(o2)}; Jolt {fc(j2)}; {fcmp(c)}')
    RES['decisions']['Q1 W8 diagnostic without the range-setting process'] = {k: yn(v) for k, v in dg.items()}

    # ================================================================================================== Q2
    P('\n## 2. Q2: C3b t_q, parent 6dd1f916 (RowWalk) vs tip 983480a9 (LeafList), blocks C (Q2C) and D (Q2D)')
    P('t_q = per-process median over [100,500) of phys_bp_query_ns, median over K; tree span = the sum of the four '
      f'phys_bp_* per-process medians. Window 6 context (tip, W1): {WIN6_TQ}')
    QB = {'C': ('Q2C',), 'D': ('Q2D',), 'pooled': ('Q2C', 'Q2D')}

    def span(r):
        v = [zone('100..500', c, 'median')(r) for c in SPAN_COLS]
        return sum(v) if all(x is not None for x in v) else None

    T = {}
    for rid, ws in (('C3b-TA-armed', (1, 8)), ('C3b-TA-pad06', (1,)), ('C3b-TD-armed', (1,))):
        for w in ws:
            for key in ('par', 'tip'):
                for bn, bl in QB.items():
                    T[(rid, key, w, bn)] = keep(f't_q {rid} {key}@W{w} [{bn}] ms',
                                                C(rid, key, w, zone('100..500', 'phys_bp_query_ns', 'median'), bl))
                sp = keep(f'tree span {rid} {key}@W{w} [pooled] ms (limit {({1: 0.36, 8: 0.35})[w]})',
                          C(rid, key, w, span, QB['pooled']))
                st = keep(f'step {rid} {key}@W{w} [pooled] [0,500) ms', C(rid, key, w, wall, QB['pooled']))
                P(f"- {rid} {key} W{w}: t_q C {fc(T[(rid, key, w, 'C')])}; D {fc(T[(rid, key, w, 'D')])}; pooled "
                  f"{fc(T[(rid, key, w, 'pooled')])}; span {fc(sp)}; step {fc(st)}")
            for bn in QB:
                c = cmp_(T[(rid, 'par', w, bn)], T[(rid, 'tip', w, bn)])
                RES['cmp'][f'Q2 t_q tip vs par {rid} W{w} [{bn}]'] = c
            P(f'  - tip vs par W{w}: ' + '; '.join(f"{bn} {fcmp(RES['cmp'][f'Q2 t_q tip vs par {rid} W{w} [{bn}]'])}"
                                                    for bn in QB))
    for key in ('par', 'tip'):
        for rid, w in (('C3b-TA-armed', 1), ('C3b-TA-armed', 8), ('C3b-TA-pad06', 1), ('C3b-TD-armed', 1)):
            c = cmp_(T[(rid, key, w, 'C')], T[(rid, key, w, 'D')])
            RES['cmp'][f'Q2 block term D vs C {rid} {key} W{w}'] = c
            P(f'- block term D vs C {rid} {key} W{w}: {fcmp(c)}')
        for bn in QB:
            c = cmp_(T[('C3b-TA-armed', key, 1, bn)], T[('C3b-TA-pad06', key, 1, bn)])
            RES['cmp'][f'Q2 layout term pad06 vs armed {key} W1 [{bn}]'] = c
            P(f'- layout term pad06 vs armed {key} W1 [{bn}]: {fcmp(c)}')
    q2 = {}
    for rid in ('C3b-TD-armed', 'C3b-TA-armed', 'C3b-TA-pad06'):
        vs = {bn: vs_bar(T[(rid, 'tip', 1, bn)], C2_BAR) for bn in QB}
        if all(v['claimed'] and v['side'] == 'above' for v in vs.values()):
            verdict = 'C2 BUILT (claimed above 0.235 in C, D and pooled)'
        elif all(v['claimed'] and v['side'] == 'below' for v in vs.values()):
            verdict = 'C2 NOT BUILT (claimed below 0.235 in C, D and pooled)'
        else:
            verdict = 'UNRESOLVED (on the bar in at least one of C, D, pooled)'
        t = T[(rid, 'tip', 1, 'pooled')]
        q2[rid] = {'verdict': verdict, 'bars': {bn: fbar(v) for bn, v in vs.items()},
                   'attr_A_0.21': fbar(vs_bar(t, ATTR_A)), 'band_0.186': fbar(vs_bar(t, BAND)),
                   'c_q_ns': t['median'] * 1e6 / ROWS_Q}
        P(f'- **C2 on {rid} (tip, W1): {verdict}**')
        for bn, v in vs.items():
            P(f'  - {bn}: {fbar(v)}')
        P(f"  - > 0.21: {q2[rid]['attr_A_0.21']}; <= 0.186: {q2[rid]['band_0.186']}; c_q = "
          f"{q2[rid]['c_q_ns']:.1f} ns/row")
    # the window-6 shift, beside this run's two blocks
    for rid in ('C3b-TA-armed', 'C3b-TA-pad06'):
        c = T[(rid, 'tip', 1, 'pooled')]
        P(f"- window 6 against this run, tip W1 {rid}: this run pooled {c['median']:.4f} [{c['min']:.4f}-{c['max']:.4f}]"
          f" = {c['median'] / 0.2102:.4f}x window 6 P3's 0.2102 and {c['median'] / 0.2354:.4f}x P5g's 0.2354 "
          '(context; different windows are never pooled)')
    RES['decisions']['Q2'] = q2

    # ================================================================================================== Q3
    P('\n## 3. Q3: the tree thresholds between 64 and 256 (recipe 1.4), instrument 93b2615b + G4_SIZES, K = 3')
    crow = w2.rowdef['G4-refine']
    ids = expected_criterion_ids(crow['args'][-1])
    sizes = sorted({int(i.rsplit('/', 1)[1]) for i in ids if '/tree_rowwalk/' not in i})
    rw_sizes = sorted({int(i.rsplit('/', 1)[1]) for i in ids if '/tree_rowwalk/' in i})

    def est(i):
        return lambda r: r['_mine']['est_ms'].get(i)
    q3 = {}
    for fam in ('uniform', 'disparity'):
        P(f'\n### {fam}')
        per = []
        for n in sizes:
            a = keep(f'Q3 {fam} all_pairs/{n}', C('G4-refine', 'g4r', 1, est(f'bp_g4_{fam}/all_pairs/{n}'), ('Q3',)))
            t = keep(f'Q3 {fam} tree/{n}', C('G4-refine', 'g4r', 1, est(f'bp_g4_{fam}/tree/{n}'), ('Q3',)))
            c = cmp_(t, a)
            RES['cmp'][f'Q3 {fam} all_pairs/tree n={n}'] = c
            slower = c['claimed'] and c['ratio'] > 1
            per.append((n, c, slower))
            P(f"- n={n}: all_pairs {fc(a, 6)}; tree {fc(t, 6)}; all_pairs/tree {fcmp(c)}"
              f"{'  <- tree claimed faster' if slower else ''}"
              f"{f'  [window 4: {WIN4_G4[fam][n]}]' if n in WIN4_G4[fam] else ''}")
        lo = max(n for n, c, s in per if not s)
        hi = min((n for n, c, s in per if s), default=None)
        mono = hi is not None and all((n >= hi) == s for n, c, s in per)
        cross = None
        for (n0, c0, _), (n1, c1, _) in zip(per, per[1:]):
            if c0['ratio'] <= 1 < c1['ratio']:
                y0, y1 = math.log(c0['ratio']), math.log(c1['ratio'])
                cross = math.exp(math.log(n0) - y0 * (math.log(n1) - math.log(n0)) / (y1 - y0))
                break
        q3[fam] = {'LO': lo, 'HI': hi, 'monotone': mono, 'loglog_crossover': cross}
        P(f'- **{fam}: LO (largest n with all_pairs not claimed slower) = {lo}; HI (smallest n with tree claimed '
          f'faster) = {hi}; monotone {mono}; log-log crossover {cross:.1f}**')
        for n in rw_sizes:
            rw = keep(f'Q3 {fam} tree_rowwalk/{n}', C('G4-refine', 'g4r', 1, est(f'bp_g4_{fam}/tree_rowwalk/{n}'), ('Q3',)))
            ll = RES['cells'][f'Q3 {fam} tree/{n}']
            ap = RES['cells'][f'Q3 {fam} all_pairs/{n}']
            c1, c2 = cmp_(rw, ll), cmp_(rw, ap)
            RES['cmp'][f'Q3 {fam} LeafList/RowWalk n={n}'] = c1
            RES['cmp'][f'Q3 {fam} all_pairs/tree_rowwalk n={n}'] = c2
            P(f'  - bridge n={n}: tree_rowwalk {fc(rw, 6)}; LeafList/RowWalk {fcmp(c1)}; all_pairs/tree_rowwalk '
              f'{c2["ratio"]:.3f} (window 4, the same kernel: {WIN4_G4[fam][n]})')
    tbmr = min(q3[f]['LO'] for f in q3)
    auto_hi = max(q3[f]['HI'] for f in q3)
    big = max(q3[f]['loglog_crossover'] for f in q3)
    mag = 10 ** (math.floor(math.log10(big)) - 1)
    l2_hi = round(big / mag) * mag
    edge = tbmr in (sizes[0], sizes[-1]) or auto_hi in (sizes[0], sizes[-1])
    q3d = {'TREE_BRUTE_MAX_ROWS': tbmr, 'AUTO_TREE_LO': tbmr, 'AUTO_TREE_HI': auto_hi, 'L2_HI': l2_hi,
           'L2_LO': round(0.9 * l2_hi), 'families': q3, 'grid_edge': 'on the grid edge' if edge else 'inside the grid'}
    RES['decisions']['Q3'] = q3d
    P(f"\n**Q3 (recipe 1.4): TREE_BRUTE_MAX_ROWS = {tbmr}; AUTO_TREE_LO / HI = {tbmr} / {auto_hi}; L2's procedure "
      f"beside it: HI {l2_hi} (the larger log-log crossover, {big:.1f}, to two significant figures), LO "
      f"{round(0.9 * l2_hi)}; {q3d['grid_edge']}**")

    # ================================================================================================== Q4
    P("\n## 4. Q4: G-TW's canary resolution on the C4 binary 989ca0f0 (J-A W1, reuse on, unarmed)")
    cp = w2.protocol['canary']
    Rg = cp['R_gtw']
    P(f"R = {100 * Rg:.3f} % (window 7's G-TW J-A W1 [0,500) two-spread bar); frac = k R / {cp['rise_over_injected']:.4f}")
    ref = keep('L9-JA-on c4@W1 (reference) [0,500) ms', C('L9-JA-on', 'c4', 1, wall, ('Q4',)))
    P(f'- reference L9-JA-on: {fc(ref)}')
    seen, q4 = {}, {}
    for r in sorted((x for x in w2.rowdef.values() if x.get('canary_of')), key=lambda x: x['canary_k']):
        k = r['canary_k']
        jc = keep(f"{r['id']} c4@W1 (frac {r['canary_frac']} = {k} R) [0,500) ms", C(r['id'], 'c4', 1, wall, ('Q4',)))
        cn = keep(f"{r['id']} injected canary ms", C(r['id'], 'c4', 1, lambda x: x['_mine']['canary_ms'], ('Q4',)))
        c = cmp_(ref, jc)
        RES['cmp'][f'Q4 canary {k}R rise'] = c
        ok = c['claimed'] and c['ratio'] > 1 and cn is not None
        seen[k] = ok
        q4[k] = {'rise_ms': c['delta'], 'rise_pct': 100 * (c['ratio'] - 1), 'injected_ms': cn['median'],
                 'expected_pct': 100 * k * Rg, 'flags': yn(c), 'seen': ok}
        P(f"- rung {k} R (frac {r['canary_frac']}, expected rise {100 * k * Rg:.2f} %): {fc(jc)}; injected {fc(cn)}; "
          f"rise {c['delta']:+.4f} ms = {100 * (c['ratio'] - 1):+.2f} % (rise/injected {c['delta'] / cn['median']:.3f}); "
          f"{fcmp(c)} -> {'SEEN' if ok else 'NOT SEEN'}")
    ks = sorted(seen)
    demo = [k for k in ks if all(seen[j] for j in ks if j >= k)]
    rk = min(demo) if demo else None
    RES['decisions']['Q4'] = {'rungs': q4, 'demonstrated': bool(seen.get(1.5) and seen.get(2.0)),
                              'resolution_k': rk, 'resolution_pct': 100 * rk * Rg if rk else None}
    P(f"**Q4: {'DEMONSTRATED' if seen.get(1.5) and seen.get(2.0) else 'NOT DEMONSTRATED'}; the smallest rung seen with "
      f"every larger rung seen: {rk} R = {100 * rk * Rg:.2f} % of the step**")

    # ================================================================================================== driver
    P("\n## 5. Against the driver's raw/reduction.json (tools/reduce7b.py)")
    drv = json.load(open(os.path.join(RAW, 'reduction.json'), encoding='utf-8'))
    ncell = ncmp = 0
    diffs = []
    for name, c in drv['cells'].items():
        mine = RES['cells'].get(name)
        if c is None and mine is None:
            continue
        ncell += 1
        if not c or not mine:
            diffs.append(('cell missing', name))
            continue
        for f in ('K', 'median', 'min', 'max'):
            if abs(c[f] - mine[f]) > 1e-9 * max(1.0, abs(c[f])):
                diffs.append(('cell', name, f, c[f], mine[f]))
    q3_worst = 0.0
    for name, c in drv['cmp'].items():
        mine = RES['cmp'].get(name)
        ncmp += 1
        if not c or not mine:
            diffs.append(('cmp missing', name))
            continue
        # Q3: the driver read criterion's printed estimate (5 significant figures); this script recomputes the
        # mean from sample.json at full precision, so Q3 ratios and medians agree to the print precision only.
        tol = 2e-4 if name.startswith('Q3 ') else 1e-9
        if name.startswith('Q3 '):
            q3_worst = max(q3_worst, abs(c['ratio'] / mine['ratio'] - 1), abs(c['A'] / mine['A'] - 1),
                           abs(c['B'] / mine['B'] - 1))
            if c['KA'] != mine['KA'] or c['KB'] != mine['KB']:
                diffs.append(('cmp K', name))
        if (abs(c['ratio'] / mine['ratio'] - 1) > tol
                or any(c[f] != mine[f] for f in ('cl_r', 'cl_i', 'cl_s', 'claimed'))):
            diffs.append(('cmp', name, c['ratio'], mine['ratio'], [c[f] for f in ('cl_r', 'cl_i', 'cl_s')],
                          [mine[f] for f in ('cl_r', 'cl_i', 'cl_s')]))
    RES['checks']['driver'] = {'cells': ncell, 'cmp': ncmp, 'diffs': diffs, 'q3_worst_rel': q3_worst}
    P(f'- {ncell} cells (K, median, min, max) and {ncmp} comparisons (ratio and the three claim flags) compared by key: '
      f'{len(diffs)} differences' + (f': {diffs[:8]}' if diffs else '')
      + f'; the 36 Q3 comparisons (whose cells the driver does not store) agree in K, medians and ratios to '
      f'{q3_worst:.1e} relative, the print precision of the estimates it read, and in every claim flag')
    dd = drv['decisions']
    agree = {
        'Q1 W8': q1[('H-trk-tree', 8)]['verdict'].split(' (')[0] in (dd['Q1 W8 headline']['verdict']),
        'Q2 C2 default row': q2['C3b-TD-armed']['verdict'].split(' (')[0] in dd['Q2 C2 C3b-TD-armed'],
        'Q2 C2 cfg-A plain': q2['C3b-TA-armed']['verdict'].split(' (')[0] in dd['Q2 C2 C3b-TA-armed'],
        'Q3': all(dd['Q3 constants'][k] == q3d[k] for k in ('TREE_BRUTE_MAX_ROWS', 'AUTO_TREE_LO', 'AUTO_TREE_HI',
                                                              'L2_HI', 'L2_LO')),
        'Q4': dd['Q4 canary']['resolution_demonstrated_k'] == rk,
    }
    RES['checks']['driver_decisions_agree'] = agree
    P(f'- decisions: {agree}')

    json.dump(RES, open(os.path.join(OUTD, 'reduction.json'), 'w', encoding='utf-8', newline='\n'), indent=1,
              default=str)
    open(os.path.join(OUTD, 'tables.txt'), 'w', encoding='utf-8', newline='\n').write('\n'.join(OUT) + '\n')
    return 0 if not diffs and all(agree.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
