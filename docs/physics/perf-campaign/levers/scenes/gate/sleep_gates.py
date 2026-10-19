#!/usr/bin/env python3
"""Dynamic parity scenes, lane SL-2b: the sleeping arm ("on") of every engine on the four programs, structural only.

usage: sleep_gates.py OUTDIR --jolt EXE --ours EXE [--rapier-dir DIR] [--legs jolt,rapier,ours] [--root DIR] [--pins PATH]
                             [--record [--write-fixtures]]
       sleep_gates.py OUTDIR --pins-from OUTDIR0 [--legs ...] [--root DIR] [--pins PATH]
       sleep_gates.py --selftest

"On" is each engine's own sleeping (01-DESIGN.md section 10): Jolt 5.6 `-allow_sleep` on the parity patch v3 build,
Rapier 0.36 `--sleeping on` on the dyn/ arms (library-default sleep on every body), ours `--sleeping on`. Programs
none (J-T, 500 steps, window 100..500), kick and shoot (800, 200..800), slide (500, 0..500). Every process runs alone, in
a fresh working directory under OUTDIR. No timing column is read: Jolt's stat line yields its hash only, `-f`'s CSV
its header and row count, and neither Rapier's nor our CSV is read past the awake columns.

Jolt leg (per program P: t1, t1b, t8, t8b, t2, t4, t16 with -pose_out, san1 -sanity, rcpt1 -receipt):
  JS1 the 9 processes exit 0; JS2 (t1) the v1 line is exactly the "on" banner (allow_sleep=1, receipt=0) and the dyn
  banner is `program=P, sanity=0, scene_dump=0, pose_out=1` (none included: -pose_out is a dyn option, critique W1);
  JS3 t1 = t1b and t8 = t8b; JS4 t2/t4/t16 against t1 (a receipt, SAME/DIFFERS: Jolt promises no W-independence);
  JS5 san1 and rcpt1 = t1; JS6 the bar (dyn_sanity.py) on san1; JS7 B1-B3 on t1's pose; JS8 one sleep line on t1 and
  on san1, program=P, may_sleep = bodies = 1240 (shoot 1300); JS9 san1's awake_discrete_th1.csv agrees with its line;
  JS10 san1's receipts line is clean; JS11 none and kick: the first all-asleep frame is 118 (pre-registered by S-SL0),
  shoot and slide: a receipt.
  JX1 / JX2 the S-SL0 bridge at -t=1 / -t=8 (J-T -f -allow_sleep -receipt -i=1000): the hash, the receipt CSV's
  sha256 and the -f CSV's shape; JX3 -scene_dump with -allow_sleep is refused with v3's message; JX4 the v1 line of
  JX1 = S-SL0's; JX5 the timed pair of the tested exe's source (its CMakeCache.txt) = v2's; JX6 the kick at 200 wakes.
Rapier leg (the 8 ruling-22 rows, frozen9b.json flags; per row and program: W1, W1b, W8, W8b, W2, W4, W16, san1, rcpt1):
  RS1 exits 0, SUMMARY void false; RS2 config.can_sleep true and sleep.can_sleep_bodies = sleep.bodies = bodies;
  RS3 W1 = W1b, W8 = W8b; RS4 W2/W4/W16 against W1 (receipt); RS5 san1 and rcpt1 = W1; RS6 the bar on san1; RS7 B1-B3
  on W1; RS8 the CSV's active_bodies column (N rows, the sleep object's figures, awake = active_bodies under --receipt);
  RS9 the liveness receipt. RX1 --scene-dump with --sleeping on exits 2 with R-7's message; RX2 the off arm (flagless
  --dyn kick) keeps can_sleep false, no sleep object, the old CSV header; RX3 a kick on an all-asleep pile wakes >= 62.
Ours leg (per program: W1, W1b, W8, W8b, W2, W4, W16, san1):
  OS1 exits 0, void_steps 0; OS2 config.sleeping true, the sleep object, frozen_v_max a number = 0; OS3 determinism;
  OS4 W2/W4/W16 = W1 (PASS/FAIL: the engine's {1, N} promise); OS5 san1 = W1; OS6 the bar; OS7 B1-B3; OS8 the CSV's
  awake column (N rows, none blank, first_frozen_step = its first zero + 1). OX1 --scene-dump with sleeping on exits 2;
  OX2 the kick wakes, as RX3.
Pins (absent under --record): per engine and program (and Rapier row) the W1 pose = pins.json `sleeping_on` fixture
  (pose_pin.check_pose: its sha256, then the bytes) and every W's printed hash = `per_w`; ours' rows also require the
  block's `recorded_with` to name the engine state it was recorded on (pins.json `ours.*`, P10's JSonT500 anchor): a
  lane that moved either and did not re-derive `sleeping_on.ours` reads them red, naming the moved value and the
  re-pin command OURS_REPIN (critique W2; 01-DESIGN.md section 10.6). A missing section or fixture is a FAIL row.
A determinism, identity or pin row whose outputs are absent FAILs (critique W1); only a W-independence receipt of an
engine that promises none, and a liveness reading, are receipts. A pin row names every run directory it reads that
left no out.txt / rc.txt (`absent output: ...`; --pins-from on an OUTDIR0 without a leg's runs gives one FAIL row per
pin and the complete tally, lane PHYS-TOOL-1).

--record skips the pin rows and writes OUTDIR/sleeping_on.json (the values; --write-fixtures also copies every W1
pose to its fixture path: scenes/fixtures/{jolt56,ours}_on_<P>.pose, D:/tmp/rapier-parity/dyn/fixtures/<arm>/
<row>_<P>_on.pose). --pins-from OUTDIR0 runs no process: the pin rows against OUTDIR0's outputs.

Exit (docs/physics/perf-campaign/GATE-KIT.md): 0 = no FAIL, the tally complete (EXPECTED below) and pass + receipts =
checks; 1 = red; 2 = could not run (usage, OUTDIR already used, an input missing: an exe, the Rapier exe directory,
frozen9b.json, dyn_sanity.py, pins.json, the v2 Jolt source, the tested Jolt exe's CMakeCache.txt or source, P10's
fidelity test), decided before the first check. Prints `checks N, pass P, receipts R, fail F`.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

G = os.path.dirname(os.path.abspath(__file__)).replace('\\', '/')
sys.path.insert(0, G)
import pose_pin  # noqa: E402  (beside this script)

RP = 'D:/tmp/rapier-parity'
FROZEN = RP + '/sweep/frozen9b.json'
# The v2 build's source (wt-v5.6.0-dyn, the tree of D:/tmp/jolt/build-v5.6.0-dyn-dist): JX5's reference timed pair.
V2_SRC = 'D:/tmp/jolt/wt-v5.6.0-dyn/PerformanceTest/PerformanceTest.cpp'
PINS_REL = 'docs/physics/perf-campaign/levers/scenes/pins.json'
FIX_REL = 'docs/physics/perf-campaign/levers/scenes/fixtures'
P10_REL = 'crates/boyko_physics/tests/v2_speculative_fidelity.rs'
PROGRAMS = ('none', 'kick', 'shoot', 'slide')
STEPS = {'none': 500, 'kick': 800, 'shoot': 800, 'slide': 500}
WINDOW = {'none': (100, 500), 'kick': (200, 800), 'shoot': (200, 800), 'slide': (0, 500)}
BODIES = {'none': 1240, 'kick': 1240, 'shoot': 1300, 'slide': 1240}
WS = (1, 2, 4, 8, 16)
KICK_STEPS = [200 + 25 * k for k in range(24)]
KICK_BODIES = 62
# = rapier_gates.Q1_ROWS: 9b's 7-row R-CLAIM union plus simd8/D0 (ruling 22 Q1)
Q1_ROWS = ('simd8/s1p7q2pd0+roff', 'simd8/s1p8q2pd0+roff', 'simd8/s1p9q2', 'simd4/s1p8q3pd0+roff',
           'block/s1p4q3pd0+roff', 'block/s1p5q2pd0+roff', 'block/s1p5q3pd0+roff', 'simd8/D0')
JOLT_ON_BANNER = 'boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=1, receipt=0'
# S-SL0 (D:/tmp/phys-orch/sl/s-sl0/REPORT.md, runs/W1_p1): Jolt 5.6 J-T -allow_sleep -receipt -i=1000 on the 9b exe
# 918fd2b7, the same at -t=1 and -t=8. The receipt CSV's sha256 is that of runs/W1_p1/receipt_discrete_th1.csv (the six
# receipt CSVs are byte-identical); the banner is runs/W1_p1/stdout.txt line 3.
S_SL0_HASH = '0xf0d442ab7603221c'
S_SL0_RECEIPT_SHA256 = 'c75a7a0ba7d5f4efabd535142c20b025d155f61ddd4ee4b38f46d5d2080e6611'
S_SL0_BANNER = 'boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=1, receipt=1'
S_SL0_STEPS = 1000
S_SL0_CSV = 'D:/tmp/phys-orch/sl/s-sl0/runs/W1_p1/receipt_discrete_th1.csv'  # --selftest only, when present
S_SL0_FIRST_ALL_ASLEEP = 118
V3_SCENE_DUMP_MSG = "-scene_dump writes the program's created values: run it without -allow_sleep"
RAPIER_SCENE_DUMP_MSG = "--scene-dump writes the program's created values: it runs with --sleeping off"
OURS_SCENE_DUMP_MSG = "--scene-dump writes the program's created values"
RECEIPT_VERDICTS = ('SAME', 'DIFFERS', 'RECEIPT', 'NOT-TESTED')
# The re-pin rule of `sleeping_on.ours` (critique W2, ruling 65): ours is not a frozen instrument, so its sleeping-on
# values hold for the engine state named by `recorded_with` only; a lane that moves that state re-derives them with:
OURS_REPIN = ('python docs/physics/perf-campaign/levers/scenes/gate/sleep_gates.py OUT --legs ours --ours EXE --record '
              '--write-fixtures, then pins.json sleeping_on.ours = OUT/sleeping_on.json ours, declared old/new')
# The tally: Jolt 4 programs x 11 + JX1-JX6; Rapier 8 rows x 4 programs x 9 + RX1-RX3; ours 4 x 8 + OX1-OX2; the pin
# rows 4 (Jolt) + 32 (Rapier) + 4 (ours). All legs with pins: 50 + 291 + 34 + 40 = 415.
EXPECTED = {'jolt': 4 * 11 + 6, 'rapier': 8 * 4 * 9 + 3, 'ours': 4 * 8 + 2}
EXPECTED_PINS = {'jolt': 4, 'rapier': 8 * 4, 'ours': 4}
GIT_ENV = {k: v for k, v in os.environ.items() if k not in ('GIT_DIR', 'GIT_WORK_TREE')}


# ── the tally ──────────────────────────────────────────────────────────────────────────────────────────────────────────

class Tally:
    def __init__(self, echo=True):
        self.rows = []
        self.echo = echo

    def add(self, name, verdict, detail=''):
        v = verdict if isinstance(verdict, str) else ('PASS' if verdict else 'FAIL')
        line = '%s\t%s\t%s' % (name, v, str(detail).replace('\t', ' ').replace('\n', ' '))
        self.rows.append(line)
        if self.echo:
            print(line, flush=True)


def verdict(rows, expected):
    """(exit, tally line): 0 iff no FAIL, `expected` rows, and every row a PASS or a known receipt."""
    vs = [r.split('\t')[1] for r in rows]
    n_pass = vs.count('PASS')
    n_fail = vs.count('FAIL')
    n_rcpt = sum(vs.count(v) for v in RECEIPT_VERDICTS)
    line = 'checks %d, pass %d, receipts %d, fail %d' % (len(rows), n_pass, n_rcpt, n_fail)
    ok = n_fail == 0 and len(rows) == expected and n_pass + n_rcpt == len(rows)
    return (0 if ok else 1), line


# ── processes ──────────────────────────────────────────────────────────────────────────────────────────────────────────

class Run:
    """One process: its exit code, its stdout + stderr, its working directory (mixed D:/ form)."""

    def __init__(self, rc, text, d):
        self.rc = rc
        self.text = text
        self.d = d

    def file(self, name):
        p = self.d + '/' + name
        if not os.path.isfile(p):
            return None
        with open(p, 'rb') as f:
            return f.read()

    def summary(self):
        s = [ln[8:] for ln in self.text.splitlines() if ln.startswith('SUMMARY ')]
        if len(s) != 1:
            return None
        try:
            return json.loads(s[0])
        except ValueError:
            return None


def run(exe, args, d):
    os.makedirs(d, exist_ok=True)
    r = subprocess.run([exe] + args, cwd=d, capture_output=True)
    text = r.stdout.decode('utf-8', 'replace') + r.stderr.decode('utf-8', 'replace')
    with open(d + '/out.txt', 'w', encoding='utf-8') as f:
        f.write(text)
    with open(d + '/rc.txt', 'w', encoding='ascii') as f:
        f.write('%d\n' % r.returncode)
    return Run(r.returncode, text, d)


def load_run(d):
    """A finished process read back from its directory (--pins-from); None when it is absent."""
    try:
        text = open(d + '/out.txt', encoding='utf-8').read()
        rc = int(open(d + '/rc.txt', encoding='ascii').read().strip())
    except (OSError, ValueError):
        return None
    return Run(rc, text, d)


def sha256_bytes(b):
    return hashlib.sha256(b).hexdigest()


def text_lines(text):
    return [ln.rstrip('\r') for ln in text.splitlines()]


# ── pure checks (each fed real-shaped and doctored inputs by --selftest) ────────────────────────────────────────────────

def same_bytes(a, b):
    """Two outputs are one: both present, non-empty and byte-equal. An absent output is never equal (critique W1)."""
    return a is not None and b is not None and len(a) > 0 and a == b


def cross_w(base, others, strict):
    """(verdict, detail) of the W-identity of `others` {W: bytes} against `base`: PASS/FAIL when the engine promises
    W-independence (`strict`), else the receipt SAME/DIFFERS; an absent output is a FAIL either way."""
    if base is None or not base or any(v is None or not v for v in others.values()):
        missing = ['W1'] if base is None or not base else []
        missing += ['W%d' % w for w, v in sorted(others.items()) if v is None or not v]
        return 'FAIL', 'absent: ' + ','.join(missing)
    det = ['W%d' % w for w, v in sorted(others.items()) if v != base]
    if strict:
        return ('PASS' if not det else 'FAIL'), ('all equal' if not det else 'differs at ' + ','.join(det))
    return ('SAME' if not det else 'DIFFERS'), ('all equal' if not det else 'differs at ' + ','.join(det))


def stat_hashes(text):
    """The hash of every Jolt stat line (`<quality>, <threads>, <rate>, 0x...`); the rate field is never kept."""
    out = []
    for ln in text_lines(text):
        parts = [p.strip() for p in ln.split(',')]
        if len(parts) == 4 and parts[0] in ('Discrete', 'LinearCast') and re.fullmatch(r'0x[0-9a-f]+', parts[3]):
            out.append(parts[3])
    return out


def one_hash(text):
    h = stat_hashes(text)
    return h[0] if len(h) == 1 else None


def kv_line(line):
    """`prefix: k=v, k=v, ...` -> {k: v}; None when a field is not k=v."""
    body = line.split(': ', 1)[1] if ': ' in line else ''
    d = {}
    for f in body.split(', '):
        if '=' not in f:
            return None
        k, v = f.split('=', 1)
        d[k] = v
    return d


def jolt_banners(text, program):
    v1 = [ln for ln in text_lines(text) if ln.startswith('boyko-parity-patch v1')]
    dyn = [ln for ln in text_lines(text) if ln.startswith('boyko-parity-dyn v1: ')]
    want = 'boyko-parity-dyn v1: program=%s, sanity=0, scene_dump=0, pose_out=1' % program
    ok = v1 == [JOLT_ON_BANNER] and dyn == [want]
    return ok, 'v1 %r dyn %r' % (v1, dyn)


SLEEP_KEYS = ('program', 'bodies', 'may_sleep', 'active_final', 'first_all_asleep', 'asleep_frames')


def jolt_sleep_line(text, program):
    """(ok, detail, fields): exactly one `boyko-parity-sleep v1:` line naming the program, every body may sleep."""
    sl = [ln for ln in text_lines(text) if ln.startswith('boyko-parity-sleep v1: ')]
    if len(sl) != 1:
        return False, '%d sleep lines' % len(sl), None
    d = kv_line(sl[0])
    if d is None or tuple(d) != SLEEP_KEYS:
        return False, 'fields %r' % sl[0], None
    try:
        f = {k: (d[k] if k == 'program' else int(d[k])) for k in SLEEP_KEYS}
    except ValueError:
        return False, 'fields %r' % sl[0], None
    ok = f['program'] == program and f['bodies'] == BODIES[program] and f['may_sleep'] == f['bodies']
    return ok, 'program=%s bodies=%d may_sleep=%d active_final=%d first_all_asleep=%d asleep_frames=%d' % tuple(
        f[k] for k in SLEEP_KEYS), f


def int_column(lines, col):
    """The int cells of column `col` of CSV data lines; None if a cell is blank or not an int."""
    out = []
    for ln in lines:
        cells = ln.split(',')
        if col >= len(cells) or not re.fullmatch(r'-?\d+', cells[col].strip()):
            return None
        out.append(int(cells[col]))
    return out


def first_zero(actives):
    return next((i for i, a in enumerate(actives) if a == 0), -1)


def jolt_awake_csv(raw, n, fields):
    """(ok, detail, actives): awake_<tag>.csv has `frame,active`, frames 0..n-1, and the sleep line's figures."""
    if raw is None:
        return False, 'no awake csv', None
    lines = [ln for ln in text_lines(raw.decode('ascii', 'replace')) if ln]
    if not lines or lines[0] != 'frame,active':
        return False, 'header %r' % (lines[:1],), None
    frames, actives = int_column(lines[1:], 0), int_column(lines[1:], 1)
    if frames is None or actives is None:
        return False, 'a cell is not an integer', None
    if frames != list(range(n)):
        return False, '%d rows, not the frames 0..%d' % (len(frames), n - 1), actives
    if fields is None:
        return False, 'no sleep line to compare with', actives
    bad = []
    if actives[-1] != fields['active_final']:
        bad.append('last %d != active_final %d' % (actives[-1], fields['active_final']))
    if actives.count(0) != fields['asleep_frames']:
        bad.append('%d zero rows != asleep_frames %d' % (actives.count(0), fields['asleep_frames']))
    if first_zero(actives) != fields['first_all_asleep']:
        bad.append('first zero %d != first_all_asleep %d' % (first_zero(actives), fields['first_all_asleep']))
    return not bad, '; '.join(bad) or '%d rows agree with the line' % n, actives


def jolt_receipts_line(text, program):
    rl = [ln for ln in text_lines(text) if ln.startswith('boyko-parity-dyn receipts: ')]
    ok = (len(rl) == 1 and ('program=%s,' % program) in rl[0] and re.search(r'\breadback_mismatches=0\b', rl[0]) is not None
          and re.search(r'\bupdate_errors=0\b', rl[0]) is not None)
    return ok, '%r' % rl


def kick_wakes(actives):
    """(applicable, short): the kick events whose previous step had nothing awake, and those that woke < 62 bodies."""
    appl = [t for t in KICK_STEPS if 1 <= t < len(actives) and actives[t - 1] == 0]
    return appl, [t for t in appl if actives[t] < KICK_BODIES]


def kick_wake_verdict(series, need_200):
    """series: {label: actives or None}. FAIL if a series is absent or a kick on an all-asleep pile woke < 62 bodies;
    NOT-TESTED if no event was applicable (and `need_200` is unset); PASS otherwise."""
    absent = [k for k, v in series.items() if v is None]
    if absent or not series:
        return 'FAIL', 'absent: %s' % ','.join(absent)
    appl, short = [], []
    for k, v in sorted(series.items()):
        a, s = kick_wakes(v)
        appl += ['%s@%d' % (k, t) for t in a]
        short += ['%s@%d (%d awake)' % (k, t, v[t]) for t in s]
    if short:
        return 'FAIL', 'woke fewer than %d: %s' % (KICK_BODIES, ', '.join(short[:6]))
    if need_200 and not any(x.endswith('@200') for x in appl):
        return 'FAIL', 'the pile was not all asleep before the kick at 200 (%d applicable)' % len(appl)
    if not appl:
        return 'NOT-TESTED', 'no kick event found the pile all asleep (0 applicable)'
    return 'PASS', '%d applicable events, each woke >= %d (%s)' % (len(appl), KICK_BODIES, ', '.join(appl[:4]))


def timed_pair(text):
    """The lines strictly between `clock_start = ...now()` and `clock_end = ...now()`; None unless each occurs once."""
    lines = text.replace('\r\n', '\n').split('\n')
    s = [i for i, ln in enumerate(lines) if 'clock_start = chrono::high_resolution_clock::now()' in ln]
    e = [i for i, ln in enumerate(lines) if 'clock_end = chrono::high_resolution_clock::now()' in ln]
    if len(s) != 1 or len(e) != 1 or s[0] >= e[0]:
        return None
    return lines[s[0] + 1:e[0]]


def jx3_verdict(rc, text, dump_written):
    ok = rc == 1 and V3_SCENE_DUMP_MSG in text and not dump_written
    return ok, 'exit %s, v3 message %s, dump written %s' % (rc, V3_SCENE_DUMP_MSG in text, dump_written)


def sl0_bridge(rc, text, receipt_raw, per_frame_raw):
    bad = []
    h = one_hash(text)
    if rc != 0:
        bad.append('exit %s' % rc)
    if h != S_SL0_HASH:
        bad.append('hash %s' % h)
    got = sha256_bytes(receipt_raw) if receipt_raw is not None else None
    if got != S_SL0_RECEIPT_SHA256:
        bad.append('receipt csv sha256 %s' % (got[:16] if got else 'absent'))
    if per_frame_raw is None:
        bad.append('no -f csv')
    else:
        pl = [ln for ln in text_lines(per_frame_raw.decode('ascii', 'replace')) if ln]
        if not pl or pl[0] != 'Frame, Time (ms), Active Bodies' or len(pl) - 1 != S_SL0_STEPS:
            bad.append('-f csv header %r, %d rows' % (pl[:1], len(pl) - 1))
    return not bad, '; '.join(bad) or 'hash %s, receipt csv = S-SL0 (%s), -f %d rows' % (h, S_SL0_RECEIPT_SHA256[:16], S_SL0_STEPS)


def rapier_on_summary(s, program):
    if s is None:
        return False, 'no SUMMARY'
    cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
    sl = s.get('sleep')
    if not isinstance(sl, dict):
        return False, 'config.can_sleep %r, no sleep object' % cfg.get('can_sleep')
    ok = cfg.get('can_sleep') is True and sl.get('can_sleep_bodies') == sl.get('bodies') == s.get('bodies') == BODIES[program]
    return ok, 'can_sleep %r, can_sleep_bodies %r, sleep.bodies %r, bodies %r' % (
        cfg.get('can_sleep'), sl.get('can_sleep_bodies'), sl.get('bodies'), s.get('bodies'))


def rapier_exits(runs):
    bad = []
    for tag, r in runs.items():
        s = r.summary() if r else None
        if r is None or r.rc != 0 or s is None or s.get('void') is not False:
            bad.append('%s rc %s void %s %s' % (tag, r.rc if r else None, s.get('void') if s else None,
                                                (s.get('voids') if s else '')))
    return not bad, '; '.join(bad[:3]) or 'all exit 0, void false'


def csv_column(raw, name):
    """(header, values) of column `name` of a CSV; values None when the column is absent or a cell is not an int."""
    if raw is None:
        return None, None
    lines = [ln for ln in text_lines(raw.decode('ascii', 'replace')) if ln]
    if not lines:
        return None, None
    hdr = lines[0].split(',')
    if name not in hdr:
        return hdr, None
    return hdr, int_column(lines[1:], hdr.index(name))


def rapier_active_check(w1_csv, rcpt_csv, s, n):
    hdr, act = csv_column(w1_csv, 'active_bodies')
    if act is None:
        return False, 'W1 csv: no integer active_bodies column (header %r)' % (hdr,)
    bad = []
    if hdr[-1] != 'active_bodies':
        bad.append('active_bodies is not the last column')
    if len(act) != n:
        bad.append('%d rows, want %d' % (len(act), n))
    sl = (s or {}).get('sleep') if isinstance((s or {}).get('sleep'), dict) else None
    if sl is None:
        bad.append('no sleep object')
    else:
        fa = first_zero(act)
        want = {'active_min': min(act) if act else None, 'active_max': max(act) if act else None,
                'first_all_asleep': fa if fa >= 0 else None, 'asleep_steps': act.count(0)}
        for k, v in want.items():
            if sl.get(k) != v:
                bad.append('sleep.%s %r != the csv\'s %r' % (k, sl.get(k), v))
    _, aw = csv_column(rcpt_csv, 'awake')
    _, ab = csv_column(rcpt_csv, 'active_bodies')
    if aw is None or ab is None or len(aw) != n or aw != ab:
        nd = sum(1 for x, y in zip(aw or [], ab or []) if x != y)
        bad.append('rcpt1: awake = active_bodies fails (%s rows, %d differ)' % (len(aw) if aw is not None else None, nd))
    return not bad, '; '.join(bad) or '%d rows; sleep object = the csv; rcpt1 awake = active_bodies' % n


def rapier_off_arm(rc, s, csv_raw):
    bad = []
    if rc != 0 or s is None:
        bad.append('exit %s, summary %s' % (rc, s is not None))
    else:
        cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
        if cfg.get('can_sleep') is not False:
            bad.append('config.can_sleep %r' % cfg.get('can_sleep'))
        if 'sleep' in s:
            bad.append('a sleep key on the off arm')
    hdr = text_lines(csv_raw.decode('ascii', 'replace'))[0] if csv_raw else None
    if hdr != 'step,wall_ns,pool_threads':
        bad.append('csv header %r' % hdr)
    return not bad, '; '.join(bad) or 'can_sleep false, no sleep key, header step,wall_ns,pool_threads'


def refusal(rc, text, want_rc, msg, dump_written):
    ok = rc == want_rc and msg in text and not dump_written
    return ok, 'exit %s (want %d), message %s, dump written %s' % (rc, want_rc, msg in text, dump_written)


def ours_exits(runs):
    bad = []
    for tag, r in runs.items():
        s = r.summary() if r else None
        if r is None or r.rc != 0 or s is None or s.get('void_steps') != 0:
            bad.append('%s rc %s void_steps %s' % (tag, r.rc if r else None, s.get('void_steps') if s else None))
    return not bad, '; '.join(bad[:3]) or 'all exit 0, void_steps 0'


def ours_on_problems(s):
    """Why a sleeping-on ours SUMMARY is not one: config.sleeping, the sleep object, frozen_v_max a number = 0."""
    if s is None:
        return ['no SUMMARY']
    cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
    why = []
    if cfg.get('sleeping') is not True:
        why.append('config.sleeping %r' % cfg.get('sleeping'))
    sl = s.get('sleep')
    if not isinstance(sl, dict):
        why.append('no sleep object')
    else:
        v = sl.get('frozen_v_max')
        if isinstance(v, bool) or not isinstance(v, (int, float)) or v != 0:
            why.append('frozen_v_max %r != 0' % (v,))
    return why


def ours_awake_check(csv_raw, s, n):
    hdr, aw = csv_column(csv_raw, 'awake')
    if aw is None:
        return False, 'no integer awake column, none blank (header %r)' % (hdr,), None
    bad = []
    if len(aw) != n:
        bad.append('%d rows, want %d' % (len(aw), n))
    fz = first_zero(aw)
    want = fz + 1 if fz >= 0 else None
    got = (s or {}).get('first_frozen_step')
    if got != want:
        bad.append('first_frozen_step %r != the csv\'s first zero + 1 = %r' % (got, want))
    return not bad, '; '.join(bad) or '%d rows, first_frozen_step %r' % (n, got), aw


def ours_stamp_problems(stamp, pins, anchor):
    """Why `sleeping_on.ours` is stale: the engine state it names is not the one pins.json and P10 name now."""
    if not isinstance(stamp, dict):
        return ['no recorded_with']
    why = []
    off = stamp.get('ours_off_pose_hash') or {}
    for p in PROGRAMS:
        now = ((pins.get('ours') or {}).get(p) or {}).get('pose_hash')
        if off.get(p) != now:
            why.append('ours.%s %s now, recorded with %s' % (p, now, off.get(p)))
    if stamp.get('p10_JSonT500') != anchor:
        why.append('P10 JSonT500 %s now, recorded with %s' % (anchor, stamp.get('p10_JSonT500')))
    return why


def pin_verdict(entry, w1_pose, fixture_abs, hashes, hash_key, absent=()):
    """(ok, detail) of one pin row: the fixture's sha256, the W1 pose's bytes, every W's hash = per_w. Every run
    directory in `absent` (a W run the row reads whose process left no out.txt / rc.txt) is named first, and FAILs
    the row (PHYS-TOOL-1: before it, --pins-from on an OUTDIR without the W1 run raised in check_pose)."""
    gone = ['absent output: %s (no out.txt/rc.txt)' % ', '.join(absent)] if absent else []
    if not isinstance(entry, dict):
        return False, '; '.join(gone + ['no pin'])
    ok, det = pose_pin.check_pose(w1_pose, fixture_abs, entry.get('fixture_sha256') or '')
    per_w = entry.get('per_w') or {}
    bad = ['W%d %s != %s' % (w, hashes.get(w), per_w.get(str(w))) for w in WS if hashes.get(w) != per_w.get(str(w))]
    if hashes.get(1) != entry.get(hash_key):
        bad.append('W1 %s != %s %s' % (hashes.get(1), hash_key, entry.get(hash_key)))
    return ok and not bad and not gone, '; '.join(gone + [det] + bad)


def p10_anchor(text):
    m = re.search(r'const ANCHOR_REUSE_ON_SLEEP_ON: u64 = (0x[0-9a-f_]+);', text)
    return m.group(1).replace('_', '') if m else None


# ── the legs ───────────────────────────────────────────────────────────────────────────────────────────────────────────

def dyn_sanity(args):
    r = subprocess.run([sys.executable, G + '/dyn_sanity.py'] + args, capture_output=True, text=True)
    return r.returncode == 0, ((r.stdout.splitlines() or [r.stderr.strip()])[0] if (r.stdout or r.stderr) else 'no output')


def bar_rows(t, ids, name, program, sanity_path, pose_path):
    """Two rows: the bar and the premises on the W1 sanity CSV, B1-B3 on the W1 pose (an absent input is a FAIL)."""
    bar, fin = ids
    if sanity_path is None or not os.path.isfile(sanity_path):
        t.add('%s %s bar B1-B6 + premise (W1)' % (bar, name), False, 'no sanity csv')
    else:
        t.add('%s %s bar B1-B6 + premise (W1)' % (bar, name), *dyn_sanity(['--program', program, sanity_path]))
    if pose_path is None or not os.path.isfile(pose_path):
        t.add('%s %s final B1-B3 (W1 pose)' % (fin, name), False, 'no pose')
    else:
        t.add('%s %s final B1-B3 (W1 pose)' % (fin, name), *dyn_sanity(['--final', program, pose_path]))


JOLT_RUNS = [('t1', 1, []), ('t1b', 1, []), ('t8', 8, []), ('t8b', 8, []), ('t2', 2, []), ('t4', 4, []), ('t16', 16, []),
             ('san1', 1, ['-sanity']), ('rcpt1', 1, ['-receipt'])]


def jolt_base(p):
    return ['-s=Pyramid'] + ([] if p == 'none' else ['-dyn=' + p]) + ['-q=Discrete', '-allow_sleep', '-i=%d' % STEPS[p]]


def jolt_leg(t, exe, src_tested, out, res):
    for p in PROGRAMS:
        runs = {tag: run(exe, jolt_base(p) + ['-t=%d' % w, '-pose_out=final.pose'] + extra, '%s/jolt/%s/%s' % (out, p, tag))
                for tag, w, extra in JOLT_RUNS}
        pose = {tag: r.file('final.pose') for tag, r in runs.items()}
        rcs = {tag: r.rc for tag, r in runs.items()}
        t.add('JS1 jolt %s exits (9 runs)' % p, all(v == 0 for v in rcs.values()),
              'all 0' if all(v == 0 for v in rcs.values()) else 'exit codes %s' % sorted(set(rcs.values())))
        t.add('JS2 jolt %s banners (t1)' % p, *jolt_banners(runs['t1'].text, p))
        ok = same_bytes(pose['t1'], pose['t1b']) and same_bytes(pose['t8'], pose['t8b'])
        t.add('JS3 jolt %s determinism t1x2 t8x2' % p, ok, '%s %s' % (one_hash(runs['t1'].text), one_hash(runs['t8'].text)))
        t.add('JS4 jolt %s cross-W pose (receipt)' % p, *cross_w(pose['t1'], {2: pose['t2'], 4: pose['t4'], 16: pose['t16']}, False))
        t.add('JS5 jolt %s -sanity / -receipt change no bit' % p, same_bytes(pose['t1'], pose['san1']) and same_bytes(pose['t1'], pose['rcpt1']),
              '%s %s' % (one_hash(runs['san1'].text), one_hash(runs['rcpt1'].text)))
        san = runs['san1'].d + '/sanity_discrete_th1.csv'
        bar_rows(t, ('JS6', 'JS7'), 'jolt %s' % p, p, san if os.path.isfile(san) else None, runs['t1'].d + '/final.pose' if pose['t1'] else None)
        ok1, d1, f1 = jolt_sleep_line(runs['t1'].text, p)
        ok2, d2, f2 = jolt_sleep_line(runs['san1'].text, p)
        t.add('JS8 jolt %s sleep line (t1, san1)' % p, ok1 and ok2 and f1 == f2, 't1 %s | san1 %s' % (d1, d2 if f1 != f2 else 'same'))
        ok, det, act = jolt_awake_csv(runs['san1'].file('awake_discrete_th1.csv'), STEPS[p], f2)
        t.add('JS9 jolt %s awake csv (san1)' % p, ok, det)
        t.add('JS10 jolt %s receipts line (san1)' % p, *jolt_receipts_line(runs['san1'].text, p))
        if f1 is None:
            t.add('JS11 jolt %s first all-asleep frame' % p, False, 'no sleep line')
        elif p in ('none', 'kick'):
            t.add('JS11 jolt %s first all-asleep frame' % p, f1['first_all_asleep'] == S_SL0_FIRST_ALL_ASLEEP,
                  '%d (S-SL0: %d)' % (f1['first_all_asleep'], S_SL0_FIRST_ALL_ASLEEP))
        else:
            t.add('JS11 jolt %s first all-asleep frame' % p, 'RECEIPT', 'first_all_asleep %d, asleep_frames %d' % (
                f1['first_all_asleep'], f1['asleep_frames']))
        res['jolt'][p] = {'runs': runs, 'fields': f1, 'actives': act}
    w1, w8 = (run(exe, ['-s=Pyramid', '-q=Discrete', '-f', '-allow_sleep', '-receipt', '-t=%d' % w, '-i=%d' % S_SL0_STEPS],
                  '%s/jolt/sl0_t%d' % (out, w)) for w in (1, 8))
    for name, r, w in (('JX1', w1, 1), ('JX2', w8, 8)):
        t.add('%s jolt S-SL0 bridge -t=%d' % (name, w), *sl0_bridge(r.rc, r.text, r.file('receipt_discrete_th%d.csv' % w),
                                                                   r.file('per_frame_discrete_th%d.csv' % w)))
    d = '%s/jolt/jx3' % out
    r = run(exe, ['-s=Pyramid', '-dyn=kick', '-q=Discrete', '-allow_sleep', '-t=1', '-i=800', '-scene_dump=d.txt'], d)
    t.add('JX3 jolt -scene_dump with -allow_sleep refused', *jx3_verdict(r.rc, r.text, os.path.exists(d + '/d.txt')))
    v1 = [ln for ln in text_lines(w1.text) if ln.startswith('boyko-parity-patch v1')]
    t.add('JX4 jolt S-SL0 v1 line', v1 == [S_SL0_BANNER], '%r' % v1)
    a, b = timed_pair(open(src_tested, encoding='utf-8').read()), timed_pair(open(V2_SRC, encoding='utf-8').read())
    t.add('JX5 jolt timed pair = v2 (%s)' % src_tested, a is not None and a == b,
          '%s lines' % (len(a) if a is not None else 'no pair') if a == b else 'differs from v2 (%s vs %s lines)' % (
              len(a) if a is not None else None, len(b) if b is not None else None))
    t.add('JX6 jolt kick wakes the asleep pile (san1)', *kick_wake_verdict({'kick': res['jolt']['kick']['actives']}, True))


RAPIER_RUNS = [('W1', 1, False), ('W1b', 1, False), ('W8', 8, False), ('W8b', 8, False), ('W2', 2, False), ('W4', 4, False),
               ('W16', 16, False), ('san1', 1, 'sanity'), ('rcpt1', 1, 'receipt')]


def rapier_args(row, p, w, d, extra):
    a = ['--cfg', row['cfg']] + list(row['flags']) + ([] if p == 'none' else ['--dyn', p])
    a += ['--sleeping', 'on', '--workers', str(w), '--steps', str(STEPS[p]), '--window', '%d..%d' % WINDOW[p],
          '--csv', d + '/run.csv', '--pose-out', d + '/final.pose']
    if extra == 'sanity':
        a += ['--sanity', d + '/sanity.csv']
    elif extra == 'receipt':
        a += ['--receipt']
    return a


def rapier_leg(t, rdir, out, res):
    by_id = {r['id']: r for r in json.load(open(FROZEN))['rows']}
    kicks = {}
    for rid in Q1_ROWS:
        row = by_id[rid]
        exe = '%s/rapier-parity-dyn-%s.exe' % (rdir, row['arm'])
        for p in PROGRAMS:
            base = '%s/rapier/%s/%s' % (out, rid.replace('/', '__'), p)
            runs = {}
            for tag, w, extra in RAPIER_RUNS:
                d = base + '/' + tag
                runs[tag] = run(exe, rapier_args(row, p, w, d, extra), d)
            pose = {tag: r.file('final.pose') for tag, r in runs.items()}
            s1 = runs['W1'].summary()
            nm = 'rapier %s %s' % (rid, p)
            t.add('RS1 %s exits and voids' % nm, *rapier_exits(runs))
            t.add('RS2 %s can_sleep on every body' % nm, *rapier_on_summary(s1, p))
            t.add('RS3 %s determinism' % nm, same_bytes(pose['W1'], pose['W1b']) and same_bytes(pose['W8'], pose['W8b']),
                  (s1 or {}).get('pose_hash'))
            t.add('RS4 %s cross-W pose (receipt)' % nm, *cross_w(pose['W1'], {2: pose['W2'], 4: pose['W4'], 16: pose['W16']}, False))
            t.add('RS5 %s --sanity / --receipt change no bit' % nm, same_bytes(pose['W1'], pose['san1']) and same_bytes(pose['W1'], pose['rcpt1']))
            san = runs['san1'].d + '/sanity.csv'
            bar_rows(t, ('RS6', 'RS7'), nm, p, san if os.path.isfile(san) else None, runs['W1'].d + '/final.pose' if pose['W1'] else None)
            t.add('RS8 %s active_bodies column' % nm, *rapier_active_check(runs['W1'].file('run.csv'), runs['rcpt1'].file('run.csv'), s1, STEPS[p]))
            sl = (s1 or {}).get('sleep')
            if isinstance(sl, dict):
                t.add('RS9 %s liveness (receipt)' % nm, 'RECEIPT', 'first_all_asleep %s, asleep_steps %s, active %s..%s' % (
                    sl.get('first_all_asleep'), sl.get('asleep_steps'), sl.get('active_min'), sl.get('active_max')))
            else:
                t.add('RS9 %s liveness (receipt)' % nm, False, 'no sleep object')
            if p == 'kick':
                kicks[rid] = csv_column(runs['W1'].file('run.csv'), 'active_bodies')[1]
            res['rapier'].setdefault(rid, {})[p] = {'runs': runs}
    row = by_id[Q1_ROWS[0]]
    exe = '%s/rapier-parity-dyn-%s.exe' % (rdir, row['arm'])
    d = '%s/rapier/rx1' % out
    r = run(exe, ['--cfg', row['cfg']] + list(row['flags']) + ['--dyn', 'kick', '--sleeping', 'on', '--workers', '1',
                                                               '--scene-dump', d + '/d.txt'], d)
    t.add('RX1 rapier --scene-dump with --sleeping on refused (%s)' % row['id'],
          *refusal(r.rc, r.text, 2, RAPIER_SCENE_DUMP_MSG, os.path.exists(d + '/d.txt')))
    d = '%s/rapier/rx2' % out
    r = run(exe, ['--cfg', row['cfg']] + list(row['flags']) + ['--dyn', 'kick', '--workers', '1', '--csv', d + '/run.csv',
                                                               '--pose-out', d + '/final.pose'], d)
    t.add('RX2 rapier off arm unchanged (%s, flagless --dyn kick)' % row['id'], *rapier_off_arm(r.rc, r.summary(), r.file('run.csv')))
    t.add('RX3 rapier kick wakes an all-asleep pile (8 rows, W1)', *kick_wake_verdict(kicks, False))


OURS_RUNS = [('W1', 1, False), ('W1b', 1, False), ('W8', 8, False), ('W8b', 8, False), ('W2', 2, False), ('W4', 4, False),
             ('W16', 16, False), ('san1', 1, True)]


def ours_args(p, w, d, sanity):
    a = ['--scene', 'jolt'] + ([] if p == 'none' else ['--dyn', p])
    a += ['--sleeping', 'on', '--workers', str(w), '--steps', str(STEPS[p]), '--window', '%d..%d' % WINDOW[p],
          '--csv', d + '/run.csv', '--pose-out', d + '/final.pose']
    return a + (['--sanity', d + '/sanity.csv'] if sanity else [])


def ours_leg(t, exe, out, res):
    kick = None
    for p in PROGRAMS:
        runs = {}
        for tag, w, san in OURS_RUNS:
            d = '%s/ours/%s/%s' % (out, p, tag)
            runs[tag] = run(exe, ours_args(p, w, d, san), d)
        pose = {tag: r.file('final.pose') for tag, r in runs.items()}
        s1 = runs['W1'].summary()
        nm = 'ours %s' % p
        t.add('OS1 %s exits and void_steps' % nm, *ours_exits(runs))
        why = ['%s: %s' % (tag, '; '.join(ours_on_problems(r.summary()))) for tag, r in runs.items() if ours_on_problems(r.summary())]
        t.add('OS2 %s sleeping on, frozen_v_max 0 (8 runs)' % nm, not why, '; '.join(why[:3]) or 'config.sleeping true, frozen_v_max 0')
        t.add('OS3 %s determinism W1x2 W8x2' % nm, same_bytes(pose['W1'], pose['W1b']) and same_bytes(pose['W8'], pose['W8b']),
              (s1 or {}).get('pose_hash'))
        t.add('OS4 %s W-identity 1/2/4/8/16' % nm, *cross_w(pose['W1'], {2: pose['W2'], 4: pose['W4'], 8: pose['W8'], 16: pose['W16']}, True))
        t.add('OS5 %s --sanity changes no bit' % nm, same_bytes(pose['W1'], pose['san1']))
        san = runs['san1'].d + '/sanity.csv'
        bar_rows(t, ('OS6', 'OS7'), nm, p, san if os.path.isfile(san) else None, runs['W1'].d + '/final.pose' if pose['W1'] else None)
        ok, det, aw = ours_awake_check(runs['W1'].file('run.csv'), s1, STEPS[p])
        t.add('OS8 %s awake column' % nm, ok, det)
        if p == 'kick':
            kick = aw
        res['ours'][p] = {'runs': runs}
    d = '%s/ours/ox1' % out
    r = run(exe, ['--scene', 'jolt', '--dyn', 'kick', '--sleeping', 'on', '--workers', '1', '--scene-dump', d + '/d.txt'], d)
    t.add('OX1 ours --scene-dump with sleeping on refused', *refusal(r.rc, r.text, 2, OURS_SCENE_DUMP_MSG, os.path.exists(d + '/d.txt')))
    t.add('OX2 ours kick wakes the asleep pile (W1)', *kick_wake_verdict({'kick': kick}, False))


# ── pins ───────────────────────────────────────────────────────────────────────────────────────────────────────────────

def read_results(out, legs):
    """The per-W hashes and W1 poses of a finished run (this one, or --pins-from's). Each entry also keeps every
    run's directory (`dirs`), so a pin row can name the runs that left no output (`absent_outputs`)."""
    res = {'jolt': {}, 'rapier': {}, 'ours': {}}

    def entry(base, tags):
        dirs = {tag: '%s/%s' % (base, tag) for tag, _, _ in tags}
        return {'runs': {tag: load_run(d) for tag, d in dirs.items()}, 'dirs': dirs}

    for p in PROGRAMS:
        if 'jolt' in legs:
            res['jolt'][p] = entry('%s/jolt/%s' % (out, p), JOLT_RUNS)
        if 'ours' in legs:
            res['ours'][p] = entry('%s/ours/%s' % (out, p), OURS_RUNS)
        if 'rapier' in legs:
            for rid in Q1_ROWS:
                res['rapier'].setdefault(rid, {})[p] = entry('%s/rapier/%s/%s' % (out, rid.replace('/', '__'), p), RAPIER_RUNS)
    return res


def w_tags(engine):
    """{W: the run tag} of the runs a pin row reads."""
    return {'jolt': {1: 't1', 2: 't2', 4: 't4', 8: 't8', 16: 't16'}}.get(engine, {1: 'W1', 2: 'W2', 4: 'W4', 8: 'W8', 16: 'W16'})


def absent_outputs(engine, r):
    """The directories of the runs a pin row reads whose process left no output (None from load_run), in W order;
    an entry without `dirs` (this invocation's own runs, never None) names the tag."""
    dirs = r.get('dirs') or {}
    return [dirs.get(tag, tag) for _, tag in sorted(w_tags(engine).items()) if r['runs'].get(tag) is None]


def hashes_of(engine, runs):
    tags = w_tags(engine)
    out = {}
    for w, tag in tags.items():
        r = runs.get(tag)
        if r is None:
            out[w] = None
        elif engine == 'jolt':
            out[w] = one_hash(r.text)
        else:
            out[w] = (r.summary() or {}).get('pose_hash')
    return out, (runs.get(tags[1]).d + '/final.pose' if runs.get(tags[1]) else None)


def rapier_fixture_rel(row, p):
    return 'dyn/fixtures/%s/%s_%s_on.pose' % (row['arm'], row['config'] + ('+roff' if row['twin'] else ''), p)


def pin_rows(t, legs, res, pins, root, anchor):
    so = pins.get('sleeping_on') if isinstance(pins.get('sleeping_on'), dict) else {}
    if 'jolt' in legs:
        for p in PROGRAMS:
            e = (so.get('jolt56') or {}).get(p)
            h, w1 = hashes_of('jolt', res['jolt'][p]['runs'])
            fx = root + '/' + e['fixture'] if isinstance(e, dict) and e.get('fixture') else ''
            gone = absent_outputs('jolt', res['jolt'][p])
            t.add('PIN jolt56 %s: W1 pose and per-W hashes = pins.json sleeping_on' % p, *pin_verdict(e, w1, fx, h, 'hash', gone))
    if 'rapier' in legs:
        for rid in Q1_ROWS:
            for p in PROGRAMS:
                e = ((so.get('rapier') or {}).get(rid) or {}).get(p)
                h, w1 = hashes_of('rapier', res['rapier'][rid][p]['runs'])
                fx = RP + '/' + e['fixture'] if isinstance(e, dict) and e.get('fixture') else ''
                gone = absent_outputs('rapier', res['rapier'][rid][p])
                t.add('PIN rapier %s %s: W1 pose and per-W hashes = pins.json sleeping_on' % (rid, p),
                      *pin_verdict(e, w1, fx, h, 'pose_hash', gone))
    if 'ours' in legs:
        ob = so.get('ours') if isinstance(so.get('ours'), dict) else {}
        stale = ours_stamp_problems(ob.get('recorded_with'), pins, anchor) if ob else ['no sleeping_on.ours']
        for p in PROGRAMS:
            e = ob.get(p)
            h, w1 = hashes_of('ours', res['ours'][p]['runs'])
            fx = root + '/' + e['fixture'] if isinstance(e, dict) and e.get('fixture') else ''
            ok, det = pin_verdict(e, w1, fx, h, 'pose_hash', absent_outputs('ours', res['ours'][p]))
            if stale:
                ok, det = False, 'stale: %s; re-derive by 01-DESIGN.md section 10.6: %s (%s)' % ('; '.join(stale), OURS_REPIN, det)
            t.add('PIN ours %s: W1 pose and per-W hashes = pins.json sleeping_on (recorded_with current)' % p, ok, det)


def record_block(legs, res, pins, root, anchor, head, ours_sha, write_fixtures):
    """The sleeping_on values from this run (OUTDIR/sleeping_on.json); with write_fixtures every W1 pose is copied to its
    fixture path and its sha256 recorded."""
    block = {}

    def fixture(src, dst_abs, rel):
        if not write_fixtures or src is None or not os.path.isfile(src):
            return {'fixture': rel, 'fixture_sha256': None}
        os.makedirs(os.path.dirname(dst_abs), exist_ok=True)
        shutil.copyfile(src, dst_abs)
        return {'fixture': rel, 'fixture_sha256': pose_pin.sha256_of(dst_abs)}

    if 'jolt' in legs:
        block['jolt56'] = {}
        for p in PROGRAMS:
            h, w1 = hashes_of('jolt', res['jolt'][p]['runs'])
            f = jolt_sleep_line(res['jolt'][p]['runs']['t1'].text, p)[2] or {}
            rel = '%s/jolt56_on_%s.pose' % (FIX_REL, p)
            e = {'hash': h[1]}
            e.update(fixture(w1, root + '/' + rel, rel))
            e.update({'per_w': {str(w): h[w] for w in WS}, 'first_all_asleep': f.get('first_all_asleep'),
                      'bodies': f.get('bodies'), 'may_sleep': f.get('may_sleep')})
            block['jolt56'][p] = e
    if 'ours' in legs:
        block['ours'] = {'recorded_with': {'tree': head, 'runner_sha256': ours_sha,
                                           'ours_off_pose_hash': {p: ((pins.get('ours') or {}).get(p) or {}).get('pose_hash') for p in PROGRAMS},
                                           'p10_JSonT500': anchor}}
        for p in PROGRAMS:
            h, w1 = hashes_of('ours', res['ours'][p]['runs'])
            s = res['ours'][p]['runs']['W1'].summary() or {}
            rel = '%s/ours_on_%s.pose' % (FIX_REL, p)
            e = {'pose_hash': h[1]}
            e.update(fixture(w1, root + '/' + rel, rel))
            e.update({'per_w': {str(w): h[w] for w in WS}, 'first_frozen_step': s.get('first_frozen_step'),
                      'frozen_v_max': (s.get('sleep') or {}).get('frozen_v_max')})
            block['ours'][p] = e
    if 'rapier' in legs:
        by_id = {r['id']: r for r in json.load(open(FROZEN))['rows']}
        block['rapier'] = {}
        for rid in Q1_ROWS:
            block['rapier'][rid] = {}
            for p in PROGRAMS:
                h, w1 = hashes_of('rapier', res['rapier'][rid][p]['runs'])
                s = res['rapier'][rid][p]['runs']['W1'].summary() or {}
                rel = rapier_fixture_rel(by_id[rid], p)
                e = {'pose_hash': h[1]}
                e.update(fixture(w1, RP + '/' + rel, rel))
                e.update({'per_w': {str(w): h[w] for w in WS}, 'first_all_asleep': (s.get('sleep') or {}).get('first_all_asleep')})
                block['rapier'][rid][p] = e
    return block


# ── main ───────────────────────────────────────────────────────────────────────────────────────────────────────────────

def cmake_source(exe):
    """The PerformanceTest.cpp of the tree the exe was configured from (CMAKE_HOME_DIRECTORY = <tree>/Build)."""
    cache = os.path.join(os.path.dirname(exe), 'CMakeCache.txt')
    if not os.path.isfile(cache):
        return None
    for ln in open(cache, 'rb').read().decode('utf-8', 'replace').splitlines():
        if ln.startswith('CMAKE_HOME_DIRECTORY:') and '=' in ln:
            home = ln.split('=', 1)[1].strip().replace('\\', '/')
            return os.path.dirname(home) + '/PerformanceTest/PerformanceTest.cpp'
    return None


def git_root(path):
    r = subprocess.run(['git', '-C', path, 'rev-parse', '--show-toplevel'], capture_output=True, text=True, env=GIT_ENV)
    return r.stdout.strip() if r.returncode == 0 else None


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument('out', nargs='?')
    ap.add_argument('--jolt')
    ap.add_argument('--ours')
    ap.add_argument('--rapier-dir', default=RP + '/dyn/bin')
    ap.add_argument('--legs', default='jolt,rapier,ours')
    ap.add_argument('--root')
    ap.add_argument('--pins')
    ap.add_argument('--record', action='store_true')
    ap.add_argument('--write-fixtures', action='store_true')
    ap.add_argument('--pins-from')
    ap.add_argument('--selftest', action='store_true')
    a = ap.parse_args(argv)
    if a.selftest:
        return selftest()
    legs = [x for x in a.legs.split(',') if x]
    if not a.out or not legs or any(x not in EXPECTED for x in legs) or len(set(legs)) != len(legs):
        print('sleep_gates.py: give OUTDIR and --legs from jolt,rapier,ours', file=sys.stderr)
        return 2
    if a.write_fixtures and not a.record:
        print('sleep_gates.py: --write-fixtures needs --record', file=sys.stderr)
        return 2
    if a.pins_from and (a.record or a.jolt or a.ours):
        print('sleep_gates.py: --pins-from runs no process: no --record, --jolt or --ours', file=sys.stderr)
        return 2
    root = git_root(a.root) if a.root else pose_pin.REPO.replace('\\', '/')
    if root is None:
        print('sleep_gates.py: --root %s is not a git checkout' % a.root, file=sys.stderr)
        return 2
    pins_path = a.pins or root + '/' + PINS_REL
    out = os.path.abspath(a.out).replace('\\', '/')
    need = [pins_path, root + '/' + P10_REL, G + '/dyn_sanity.py']
    src = None
    if not a.pins_from:
        if 'jolt' in legs:
            need += [a.jolt or '(--jolt)', V2_SRC]
            src = cmake_source(a.jolt) if a.jolt and os.path.isfile(a.jolt) else None
            need.append(src or ('(the source of %s: no CMakeCache.txt with CMAKE_HOME_DIRECTORY beside it)' % a.jolt))
        if 'ours' in legs:
            need.append(a.ours or '(--ours)')
        if 'rapier' in legs:
            need += [FROZEN] + ['%s/rapier-parity-dyn-%s.exe' % (a.rapier_dir, arm) for arm in ('simd8', 'simd4', 'block')]
    else:
        need.append(a.pins_from)
        if 'rapier' in legs:
            need.append(FROZEN)
    missing = [p for p in need if not os.path.exists(p)]
    if missing:
        print('sleep_gates.py: missing input %s' % ', '.join(missing), file=sys.stderr)
        return 2
    if os.path.exists(out + '/gates.tsv'):
        print('sleep_gates.py: %s was already used (gates.tsv): every process needs a fresh directory' % out, file=sys.stderr)
        return 2
    os.makedirs(out, exist_ok=True)
    pins = json.load(open(pins_path, encoding='utf-8'))
    anchor = p10_anchor(open(root + '/' + P10_REL, encoding='utf-8').read())
    t = Tally()
    if a.pins_from:
        res = read_results(os.path.abspath(a.pins_from).replace('\\', '/'), legs)
        expected = sum(EXPECTED_PINS[x] for x in legs)
    else:
        res = {'jolt': {}, 'rapier': {}, 'ours': {}}
        if 'jolt' in legs:
            jolt_leg(t, a.jolt, src, out, res)
        if 'rapier' in legs:
            rapier_leg(t, a.rapier_dir, out, res)
        if 'ours' in legs:
            ours_leg(t, a.ours, out, res)
        expected = sum(EXPECTED[x] for x in legs) + (0 if a.record else sum(EXPECTED_PINS[x] for x in legs))
    if not a.record:
        pin_rows(t, legs, res, pins, root, anchor)
    else:
        head = subprocess.run(['git', '-C', root, 'rev-parse', 'HEAD'], capture_output=True, text=True, env=GIT_ENV).stdout.strip()
        ours_sha = pose_pin.sha256_of(a.ours) if 'ours' in legs else None
        block = record_block(legs, res, pins, root, anchor, head, ours_sha, a.write_fixtures)
        with open(out + '/sleeping_on.json', 'w', encoding='utf-8') as f:
            json.dump(block, f, indent=1)
        print('sleeping_on values: %s/sleeping_on.json (fixtures %s)' % (out, 'written' if a.write_fixtures else 'not written'))
    with open(out + '/gates.tsv', 'w', encoding='utf-8') as f:
        f.write('\n'.join(t.rows) + '\n')
    rc, line = verdict(t.rows, expected)
    print(line + ' (expected %d)' % expected)
    return rc


# ── selftest: every check fed a real-shaped input (green) and a doctored one (red for its reason) ──────────────────────

def guarded(fn):
    """fn()'s value, or the exception it raised as a string: a raise is the case's FAIL, named, not the selftest's
    crash (PHYS-TOOL-1's mutation M2a: check_pose raised on a None pose)."""
    try:
        return fn()
    except Exception as ex:  # noqa: BLE001 - every raise is a red case
        return 'raised %s: %s' % (type(ex).__name__, ex)


def committed_on_pin(program):
    """(the committed fixture's absolute path, its pinned sha256) of `sleeping_on.ours.<program>` in this checkout."""
    root = pose_pin.REPO.replace('\\', '/')
    e = json.load(open(root + '/' + PINS_REL, encoding='utf-8'))['sleeping_on']['ours'][program]
    return root + '/' + e['fixture'], e['fixture_sha256']


def absent_end_to_end():
    """main() with --pins-from an empty OUTDIR0 (legs jolt,ours) in temp dirs: (exit, rows, every row a FAIL naming an
    absent output, the tally line)."""
    import contextlib
    import io
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        os.makedirs(td + '/out0')
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = main([td + '/out', '--pins-from', td + '/out0', '--legs', 'jolt,ours'])
        rows = open(td + '/out/gates.tsv', encoding='utf-8').read().splitlines()
    named = all(r.split('\t')[1] == 'FAIL' and 'absent output: ' in r for r in rows)
    return rc, len(rows), named, buf.getvalue().splitlines()[-1]


def selftest():
    cases = []

    def case(name, got, want):
        cases.append((name, got == want, got, want))

    p = 'shoot'
    ban = '\n'.join(['Running scene: Pyramid', JOLT_ON_BANNER, 'boyko-parity-dyn v1: program=shoot, sanity=0, scene_dump=0, pose_out=1'])
    case('banners: the on banner and the dyn line', jolt_banners(ban, p)[0], True)
    case('banners: allow_sleep=0 is red', jolt_banners(ban.replace('allow_sleep=1', 'allow_sleep=0'), p)[0], False)
    case('banners: none needs its dyn line (W1)', jolt_banners(JOLT_ON_BANNER, 'none')[0], False)
    case('banners: none with program=none', jolt_banners(JOLT_ON_BANNER + '\nboyko-parity-dyn v1: program=none, sanity=0, scene_dump=0, pose_out=1', 'none')[0], True)
    sl = 'boyko-parity-sleep v1: program=shoot, bodies=1300, may_sleep=1300, active_final=0, first_all_asleep=700, asleep_frames=100'
    case('sleep line: one, every body may sleep', jolt_sleep_line(sl, p)[0], True)
    case('sleep line: two lines are red', jolt_sleep_line(sl + '\n' + sl, p)[0], False)
    case('sleep line: may_sleep 1299 of 1300 is red', jolt_sleep_line(sl.replace('may_sleep=1300', 'may_sleep=1299'), p)[0], False)
    case('sleep line: another program is red', jolt_sleep_line(sl.replace('program=shoot', 'program=kick'), p)[0], False)
    case('sleep line: none is red', jolt_sleep_line('', p)[0], False)
    case('sleep line: its last field is a decimal (the stat-line hash reader skips it)', re.search(r'0x[0-9a-f]*$', sl), None)
    f = jolt_sleep_line(sl, p)[2]
    acts = [1240] * 700 + [0] * 100
    good = ('frame,active\n' + ''.join('%d,%d\n' % (i, a) for i, a in enumerate(acts))).encode()
    case('awake csv: agrees with the line', jolt_awake_csv(good, 800, f)[0], True)
    case('awake csv: 799 rows is red', jolt_awake_csv(good.rsplit(b'\n', 2)[0] + b'\n', 800, f)[0], False)
    wrong_last = ('frame,active\n' + ''.join('%d,%d\n' % (i, a) for i, a in enumerate(acts[:-1] + [3]))).encode()
    case('awake csv: a wrong last row is red', jolt_awake_csv(wrong_last, 800, f)[0], False)
    f2 = dict(f, asleep_frames=99)
    case('awake csv: a wrong zero count is red', jolt_awake_csv(good, 800, f2)[0], False)
    case('awake csv: a NaN cell is red', jolt_awake_csv(good.replace(b'\n5,1240\n', b'\n5,nan\n'), 800, f)[0], False)
    case('awake csv: absent is red', jolt_awake_csv(None, 800, f)[0], False)
    recp = b'Frame, Manifolds, Points, Active Bodies, Top Y\n0, 1, 2, 3, 4\n'
    pf = b'Frame, Time (ms), Active Bodies\n' + b'0, 1, 2\n' * 1000
    st = 'Discrete, 1, 1.0, %s' % S_SL0_HASH
    case('S-SL0 bridge: a receipt csv other than S-SL0\'s is red', sl0_bridge(0, st, recp, pf)[0], False)
    if os.path.isfile(S_SL0_CSV):
        real = open(S_SL0_CSV, 'rb').read()
        case('S-SL0 bridge: S-SL0\'s own receipt csv (on disk)', sl0_bridge(0, st, real, pf)[0], True)
        case('S-SL0 bridge: its receipt csv with one byte changed is red',
             sl0_bridge(0, st, real[:100] + bytes([real[100] ^ 1]) + real[101:], pf)[0], False)
        case('S-SL0 bridge: 999 -f rows is red', sl0_bridge(0, st, real, pf[:-len(b'0, 1, 2\n')])[0], False)
        case('S-SL0 bridge: another hash is red', sl0_bridge(0, 'Discrete, 1, 1.0, 0x1', real, pf)[0], False)
    else:
        print('selftest: %s absent: the bridge\'s four on-disk cases are skipped' % S_SL0_CSV)
    case('S-SL0 bridge: the hash is read from the stat line only', one_hash('Discrete, 1, 148.1, 0xabc\n' + sl), '0xabc')
    src = 'x\n\t\tclock_start = chrono::high_resolution_clock::now();\n\t\ta();\n\t\tb();\n\t\tclock_end = chrono::high_resolution_clock::now();\n'
    case('timed pair: equal', timed_pair(src) == timed_pair(src.replace('\n', '\r\n')), True)
    case('timed pair: one extra statement is red', timed_pair(src) == timed_pair(src.replace('b();', 'b();\n\t\tc();')), False)
    case('timed pair: no clock is red', timed_pair('a\nb\n'), None)
    case('JX3: exit 0 is red', jx3_verdict(0, '', False)[0], False)
    case('JX3: exit 1 with v2 message is red', jx3_verdict(1, '-dyn, ... run on the Pyramid scene without -allow_sleep only', False)[0], False)
    case('JX3: exit 1 with v3 message', jx3_verdict(1, V3_SCENE_DUMP_MSG + ' (boyko...)', False)[0], True)
    case('JX3: a dump written is red', jx3_verdict(1, V3_SCENE_DUMP_MSG, True)[0], False)
    rs = {'bodies': 1300, 'void': False, 'config': {'can_sleep': True},
          'sleep': {'can_sleep_bodies': 1300, 'bodies': 1300, 'active_min': 0, 'active_max': 1300, 'first_all_asleep': 700, 'asleep_steps': 100}}
    case('rapier summary: sleep-capable', rapier_on_summary(rs, p)[0], True)
    case('rapier summary: can_sleep false is red', rapier_on_summary(dict(rs, config={'can_sleep': False}), p)[0], False)
    case('rapier summary: can_sleep_bodies 1299 is red', rapier_on_summary(dict(rs, sleep=dict(rs['sleep'], can_sleep_bodies=1299)), p)[0], False)
    case('rapier summary: no sleep object is red', rapier_on_summary({k: v for k, v in rs.items() if k != 'sleep'}, p)[0], False)
    good_run = Run(0, 'SUMMARY ' + json.dumps(rs), '')
    case('rapier exits: void false', rapier_exits({'W1': good_run})[0], True)
    case('rapier exits: void true is red', rapier_exits({'W1': Run(0, 'SUMMARY ' + json.dumps(dict(rs, void=True)), '')})[0], False)
    case('rapier exits: absent run is red', rapier_exits({'W1': None})[0], False)
    hdr = 'step,wall_ns,pool_threads,active_bodies\n'
    w1csv = (hdr + ''.join('%d,9,1,%d\n' % (i, a) for i, a in enumerate([1300] * 700 + [0] * 100))).encode()
    rhdr = 'step,wall_ns,pool_threads,awake,top_y,active_bodies\n'
    rc_good = (rhdr + ''.join('%d,9,1,%d,1.0,%d\n' % (i, a, a) for i, a in enumerate([1300] * 700 + [0] * 100))).encode()
    case('rapier active_bodies: the column and the sleep object agree', rapier_active_check(w1csv, rc_good, rs, 800)[0], True)
    rc_bad = rc_good.replace(b'\n10,9,1,1300,1.0,1300\n', b'\n10,9,1,1299,1.0,1300\n')
    case('rapier active_bodies: awake != active_bodies on one row is red', rapier_active_check(w1csv, rc_bad, rs, 800)[0], False)
    case('rapier active_bodies: a sleep object off the csv is red',
         rapier_active_check(w1csv, rc_good, dict(rs, sleep=dict(rs['sleep'], first_all_asleep=699)), 800)[0], False)
    case('rapier off arm: header and no sleep key', rapier_off_arm(0, {'config': {'can_sleep': False}}, b'step,wall_ns,pool_threads\n0,1,1\n')[0], True)
    case('rapier off arm: a sleep key is red', rapier_off_arm(0, {'config': {'can_sleep': False}, 'sleep': {}}, b'step,wall_ns,pool_threads\n')[0], False)
    case('rapier off arm: a new column is red', rapier_off_arm(0, {'config': {'can_sleep': False}}, b'step,wall_ns,pool_threads,active_bodies\n')[0], False)
    os_ = {'void_steps': 0, 'config': {'sleeping': True}, 'sleep': {'frozen_v_max': 0}, 'first_frozen_step': 147}
    case('ours on: frozen_v_max 0', ours_on_problems(os_), [])
    case('ours on: frozen_v_max 2.4e-6 is red', bool(ours_on_problems(dict(os_, sleep={'frozen_v_max': 2.4e-6}))), True)
    case('ours on: frozen_v_max null is red', bool(ours_on_problems(dict(os_, sleep={'frozen_v_max': None}))), True)
    case('ours on: no sleep object is red', bool(ours_on_problems({k: v for k, v in os_.items() if k != 'sleep'})), True)
    case('ours on: config.sleeping false is red', bool(ours_on_problems(dict(os_, config={'sleeping': False}))), True)
    ocsv = ('step,wall_ns,manifolds,pairs,top_y,awake\n' + ''.join('%d,9,1,1,1.0,%d\n' % (i, a) for i, a in enumerate([1240] * 146 + [0] * 54 + [1240] * 600))).encode()
    case('ours awake: first_frozen_step = first zero + 1', ours_awake_check(ocsv, os_, 800)[0], True)
    case('ours awake: a blank cell is red', ours_awake_check(ocsv.replace(b'\n3,9,1,1,1.0,1240\n', b'\n3,9,1,1,1.0,\n'), os_, 800)[0], False)
    case('ours awake: first_frozen_step off by one is red', ours_awake_check(ocsv, dict(os_, first_frozen_step=146), 800)[0], False)
    a1, b1 = b'pose-a', b'pose-b'
    case('cross-W: ours W16 differs is red', cross_w(a1, {2: a1, 16: b1}, True)[0], 'FAIL')
    case('cross-W: Jolt W16 differs is a receipt', cross_w(a1, {2: a1, 16: b1}, False)[0], 'DIFFERS')
    case('cross-W: an absent W is red even as a receipt (W1)', cross_w(a1, {2: None}, False)[0], 'FAIL')
    case('cross-W: an absent W1 is red', cross_w(None, {2: None}, False)[0], 'FAIL')
    case('determinism: a second run differing is red', same_bytes(a1, b1), False)
    case('determinism: two absent outputs are red (W1)', same_bytes(None, None), False)
    case('determinism: two empty outputs are red', same_bytes(b'', b''), False)
    ka = [1240] * 118 + [0] * 82 + [1240] * 600
    case('kick wakes: 1240 woken', kick_wake_verdict({'k': ka}, True)[0], 'PASS')
    case('kick wakes: 61 woken is red', kick_wake_verdict({'k': ka[:200] + [61] + ka[201:]}, True)[0], 'FAIL')
    case('kick wakes: never asleep is NOT-TESTED', kick_wake_verdict({'k': [1240] * 800}, False)[0], 'NOT-TESTED')
    case('kick wakes: never asleep is red where S-SL0 pre-registers it', kick_wake_verdict({'k': [1240] * 800}, True)[0], 'FAIL')
    case('kick wakes: an absent series is red', kick_wake_verdict({'k': None}, False)[0], 'FAIL')
    good_rows = ['a\tPASS\t', 'b\tSAME\t', 'c\tRECEIPT\t']
    case('tally: complete', verdict(good_rows, 3)[0], 0)
    case('tally: a dropped entry is red', verdict(good_rows[:2], 3)[0], 1)
    case('tally: an extra entry is red', verdict(good_rows + ['d\tPASS\t'], 3)[0], 1)
    case('tally: an unknown verdict is red', verdict(good_rows[:2] + ['c\tMAYBE\t'], 3)[0], 1)
    case('tally: a FAIL is red', verdict(good_rows[:2] + ['c\tFAIL\t'], 3)[0], 1)
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        fx, ps = td + '/fx.pose', td + '/w1.pose'
        open(fx, 'wb').write(b'0123456789ab' * 13)
        shutil.copyfile(fx, ps)
        sha = pose_pin.sha256_of(fx)
        hs = {w: '0x%x' % w for w in WS}
        e = {'pose_hash': hs[1], 'fixture_sha256': sha, 'per_w': {str(w): hs[w] for w in WS}}
        case('pin: fixture, bytes and per-W hashes', pin_verdict(e, ps, fx, hs, 'pose_hash')[0], True)
        case('pin: a wrong fixture sha256 is red', pin_verdict(dict(e, fixture_sha256='0' * 64), ps, fx, hs, 'pose_hash')[0], False)
        case('pin: a wrong per_w hash is red', pin_verdict(dict(e, per_w=dict(e['per_w'], **{'8': '0xdead'})), ps, fx, hs, 'pose_hash')[0], False)
        case('pin: no pin is red', pin_verdict(None, ps, fx, hs, 'pose_hash')[0], False)
        case('pin: no pose is red', pin_verdict(e, td + '/absent.pose', fx, hs, 'pose_hash')[0], False)
        w1_dir = td + '/out0/ours/kick/W1'
        got = guarded(lambda: pin_verdict(e, None, fx, {**hs, 1: None}, 'pose_hash', [w1_dir]))
        case('pin: an absent W1 names its directory (PHYS-TOOL-1)',
             (got[0], 'absent output: %s' % w1_dir in got[1]) if isinstance(got, tuple) else got, (False, True))
    committed = guarded(lambda: committed_on_pin('kick'))
    got = guarded(lambda: pose_pin.check_pose(None, *committed)) if isinstance(committed, tuple) else committed
    case('check_pose: a None pose is a FAIL naming "no pose", never a raise (PHYS-TOOL-1)',
         (got[0], 'no pose' in got[1]) if isinstance(got, tuple) else got, (False, True))
    case('end to end: --pins-from an empty OUTDIR0, legs jolt,ours: exit 1, 8 FAIL rows naming the absent outputs, '
         'the complete tally (PHYS-TOOL-1)', guarded(absent_end_to_end),
         (1, 8, True, 'checks 8, pass 0, receipts 0, fail 8 (expected 8)'))
    pins = {'ours':{q: {'pose_hash': '0x%d' % i} for i, q in enumerate(PROGRAMS)}}
    stamp = {'ours_off_pose_hash': {q: '0x%d' % i for i, q in enumerate(PROGRAMS)}, 'p10_JSonT500': '0xc637'}
    case('ours stamp: current', ours_stamp_problems(stamp, pins, '0xc637'), [])
    case('ours stamp: a moved off-arm pin is stale (W2)', bool(ours_stamp_problems(stamp, {'ours': dict(pins['ours'], kick={'pose_hash': '0x9'})}, '0xc637')), True)
    case('ours stamp: a moved P10 anchor is stale (W2)', bool(ours_stamp_problems(stamp, pins, '0x1')), True)
    case('P10 anchor parse', p10_anchor('const ANCHOR_REUSE_ON_SLEEP_ON: u64 = 0xc637_a051_7ada_bab7;'), '0xc637a0517adabab7')
    case('refusal: exit 2 with the message', refusal(2, 'x ' + RAPIER_SCENE_DUMP_MSG, 2, RAPIER_SCENE_DUMP_MSG, False)[0], True)
    case('refusal: unknown flag (the old exe) is red', refusal(2, 'rapier-parity: unknown flag "--sleeping"', 2, RAPIER_SCENE_DUMP_MSG, False)[0], False)
    case('expected: 415 for every leg with pins', sum(EXPECTED.values()) + sum(EXPECTED_PINS.values()), 415)
    n_pass = sum(1 for c in cases if c[1])
    for name, ok, got, want in cases:
        if not ok:
            print('selftest FAIL %s: got %r, want %r' % (name, got, want))
    print('selftest %d cases, %d pass' % (len(cases), n_pass))
    return 0 if n_pass == len(cases) else 1


if __name__ == '__main__':
    sys.exit(main())
