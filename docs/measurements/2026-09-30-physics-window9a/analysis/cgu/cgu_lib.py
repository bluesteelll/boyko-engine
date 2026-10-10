"""Window 9a (resumed 2026-10-01, run tag 033740), block C4-CGU: codegen-units 1 vs 16 on the tip default.
results-analyst, 2026-10-01. Read-only on raw/, rows9a*.json, bin/, gate/fixtures/.

Reuses analysis/c4ab/lib9a.py UNCHANGED for: the per-process parser and independent validity check
(load_process: SUMMARY, run.csv, pose bytes == fixture, config receipt, TreeDiag, workers, sleeping off,
window mean vs SUMMARY), clean_why (ruling 8's clean definition), and the statistics (cell / cmp_ / ruling1 /
judge: i = IQR/median, s = 1.2533*SD/sqrt(K)/median, r = range/median; flag iff |B/A-1| > 2*hypot(A_x,B_x);
CLAIMED iff i AND s pooled AND in every gating pass with one sign; STRONG iff r too).

lib9a hard-codes RUN_TAG = '191354' inside closed_passes/procs_of; this file re-implements those two with the
run tag as a parameter (same predicate otherwise) so the resumed run (033740) and C4-AB (191354) are both read.

Convention: A = tip (cgu 16, the existing parity exe), B = tipcgu1 (cgu 1). B/A < 1 => cgu1 faster.
Primary metric window [100,500) (orchestrator addendum 2026-10-01, pre-registered before the resume);
[0,500) reported beside it, deciding nothing.
"""
import hashlib
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'c4ab'))
import lib9a as L  # noqa: E402

W9A = L.W9A
RAW = L.RAW
RUN_CGU = '033740'
RUN_AB = '191354'
BLOCK = 'C4-CGU'
ROW = 'C4-JDcgu'
A_KEY, B_KEY = 'tip', 'tipcgu1'
WS = (1, 2, 4, 8, 16)
PRIMARY = '100..500'
BESIDE = '0..500'
PIN = {'tip': '8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3',
       'tipcgu1': '3e1f72babf76987260988481f12ddbe409ecdc308ce141326529ef89b020d771'}


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write('\n'.join(self.lines) + '\n')


def all_records():
    return L.all_records()


def closed_passes(recs, run_tag):
    return {(r['block'], r['pass'], r['pass_attempt']) for r in recs
            if r.get('pass_done') and r.get('run_tag') == run_tag}


def procs_of(block, recs, run_tag):
    cp = closed_passes(recs, run_tag)
    return [r for r in recs if r.get('block') == block and 'row' in r and r.get('attempt') != 'warmup'
            and r.get('timed', True) and r.get('run_tag') == run_tag
            and (r['block'], r['pass'], r['pass_attempt']) in cp]


def select(block, recs, run_tag, row=None, binary=None):
    """lib9a.select with the run tag as a parameter. Slot = (block, pass, seq): the original if clean and valid,
    else the FIRST clean valid re-run (rerun_no order), else dropped."""
    procs = [L.load_process(r) for r in procs_of(block, recs, run_tag)
             if (row is None or r['row'] == row) and (binary is None or r['binary'] == binary)]
    slots = {}
    for p in procs:
        slots.setdefault((p['block'], p['pass'], p['seq']), []).append(p)
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


def vals(used, binary, W, win, row=ROW):
    """{pass: [values]} of the used processes of one cell."""
    out = {}
    for p in used:
        if p['row'] == row and p['binary'] == binary and p['W'] == W:
            out.setdefault(p['pass'], []).append(p['v'][win])
    return out


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()
