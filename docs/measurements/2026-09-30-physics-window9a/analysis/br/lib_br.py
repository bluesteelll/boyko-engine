"""Window 9a, block C4-BR (the all_pairs bracket, K 3 = 1 pass x 3 rounds, DIAGNOSTIC ONLY by ruling (b)):
analysis library (results-analyst, 2026-10-01). Read-only on raw/, rows9a.json, bin/SHA256SUMS.

Data: ONLY the resumed run (run_tag 033740) and only a pass with a pass_done record. The aborted 2026-09-30 attempt
(run_tag 191354, raw/C4-BR-p0_191354) is NOT data and is only counted.

Per-process value (window 7 wave 2's and window 8b's G4 reader, verbatim): for each criterion id, the mean over the
10 Flat samples of times/iters from criterion/<id>/new/sample.json, in us. The printed estimate (stdout) and the
driver's est_ms are compared against it, never used.

Statistics: lib9a (= window 8b synthlib = window 8 lib8) imported unchanged: cell = median over K; i = IQR (inclusive)
/ median; s = 1.2533 SD / sqrt(K) / median; r = (max - min) / median; flag x iff |B/A - 1| > 2 hypot(A_x, B_x), K >= 3.
With ONE pass the pass is the pooled cell (window 8b's F3-G4 convention). Ruling (b): the block is DIAGNOSTIC ONLY,
so i AND s here is reported as RESOLVED (diagnostic), never as CLAIMED.
"""
import json
import os
import re
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W9A, 'raw')
sys.path.insert(0, os.path.join(W9A, 'analysis', 'c4ab'))
import lib9a as L  # noqa: E402  cell, cmp_, yn, clean_why, all_records, SUMS, BINS

RUN_TAG = '033740'
BLOCK = 'C4-BR'
ROW = L.ROWS['C4-BR']
NEED = sorted(ROW['expect'])
BINARIES = ('g4r7', 'g4r8b', 'g4rT')
FAMS = ('uniform', 'disparity')
SIZES = (144, 256)
ARMS = ('all_pairs', 'tree')
TIME_RE = re.compile(r'^(bp_g4_\S+)\s*\n?\s*time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]', re.M)
UNIT_US = {'ns': 1e-3, 'ms': 1e3, 's': 1e6, 'ps': 1e-6}
KRE = re.compile(r'^bp_g4_(\w+)/(\d+): kernel (\w+) rows (\d+) pairs (\d+) ', re.M)


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write('\n'.join(self.lines) + '\n')


def unit_us(u):
    if u in UNIT_US:
        return UNIT_US[u]
    if len(u) == 2 and u.endswith('s'):  # the micro sign, in any encoding
        return 1.0
    raise ValueError(u)


def read_text(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def recs_all():
    return L.all_records()


def br_records(recs, tag=RUN_TAG):
    return [r for r in recs if r.get('block') == BLOCK and r.get('run_tag') == tag and 'row' in r]


def closed(recs, tag=RUN_TAG):
    return {(r['pass'], r['pass_attempt']) for r in recs
            if r.get('pass_done') and r.get('block') == BLOCK and r.get('run_tag') == tag}


def load(r):
    """Independent validity + payload for one criterion process."""
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('hang'):
        why.append('hang')
    if not L.sha_ok(r):
        why.append('sha256')
    so = read_text(os.path.join(d, 'stdout.txt'))
    se = read_text(os.path.join(d, 'stderr.txt'))
    printed = {}
    for m in TIME_RE.finditer(so):
        printed.setdefault(m.group(1), []).append(float(m.group(4)) * unit_us(m.group(5)))
    est, nsamp, modes = {}, {}, {}
    for i in NEED:
        sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
        if not os.path.exists(sp):
            continue
        s = json.load(open(sp, encoding='utf-8'))
        modes[i] = s.get('sampling_mode')
        nsamp[i] = len(s['times'])
        if s.get('sampling_mode') != 'Flat' or len(s['times']) != 10:
            why.append('%s sampling %s x%d' % (i, s.get('sampling_mode'), len(s['times'])))
        est[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3  # ns -> us
    on_disk = set()
    for root, dirs, files in os.walk(os.path.join(d, 'criterion')):
        if os.path.basename(root) == 'new' and 'benchmark.json' in files:
            on_disk.add(json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8'))['full_id'])
    change_dirs = [root for root, dirs, files in os.walk(os.path.join(d, 'criterion'))
                   if os.path.basename(root) == 'change']
    miss = [i for i in NEED if i not in est or i not in printed]
    extra = sorted((set(printed) | on_disk) - set(NEED))
    dup = [i for i, v in printed.items() if len(v) != 1]
    if miss:
        why.append('missing %s' % miss)
    if extra:
        why.append('extra %s' % extra)
    if dup:
        why.append('dup %s' % dup)
    if change_dirs:
        why.append('criterion compared against a previous baseline (%d change dirs)' % len(change_dirs))
    worst_print = max((abs(printed[i][0] / est[i] - 1) for i in est if i in printed), default=None)
    if worst_print is None or worst_print > 1e-3:
        why.append('printed vs sample.json %s' % worst_print)
    drv = (r.get('criterion') or {}).get('est_ms') or {}
    worst_drv = max((abs(drv[i][1] * 1e3 / est[i] - 1) for i in est if i in drv), default=None)
    if worst_drv is None or worst_drv > 1e-3:
        why.append('driver est_ms vs sample.json %s' % worst_drv)
    work = {}
    for fam, n, kern, rows_, pairs in KRE.findall(se):
        if int(n) in SIZES:
            work[(fam, int(n), kern)] = (int(rows_), int(pairs))
    perf = [x['total'] for x in (r.get('perf') or []) if x.get('total') is not None]
    return {'rec': r, 'binary': r['binary'], 'round': r['round'], 'seq': r['seq'], 'pass': r['pass'],
            'attempt': r['attempt'], 'rerun_no': r.get('rerun_no', 0), 'est': est, 'printed': printed,
            'nsamp': nsamp, 'modes': modes, 'on_disk': on_disk, 'worst_print': worst_print, 'worst_drv': worst_drv,
            'work': work, 'perf_med': statistics.median(perf) if perf else None, 'perf_n': len(perf),
            'valid_why': why, 'clean_why': L.clean_why(r)}


def select(recs):
    """Ruling 8's slot rule over the closed pass(es) of the resumed run. Slot = (pass, seq)."""
    cp = closed(recs)
    procs = [load(r) for r in br_records(recs) if (r['pass'], r['pass_attempt']) in cp and r.get('timed', True)
             and r.get('attempt') != 'warmup']
    slots = {}
    for p in procs:
        slots.setdefault((p['pass'], p['seq']), []).append(p)
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


def ident(fam, arm, n):
    return 'bp_g4_%s/%s/%d' % (fam, arm, n)


def cells(used):
    out = {}
    for b in BINARIES:
        for i in NEED:
            out[(b, i)] = L.cell([p['est'][i] for p in used if p['binary'] == b])
    return out


def diag_label(c):
    """K 3 one pass, ruling (b): i AND s = RESOLVED (diagnostic); +r noted. Never CLAIMED."""
    if c['ratio'] is None:
        return 'n/a'
    if c['cl_i'] and c['cl_s']:
        return 'RESOLVED+r (diag)' if c['cl_r'] else 'RESOLVED (diag)'
    return 'not resolved (diag)'
