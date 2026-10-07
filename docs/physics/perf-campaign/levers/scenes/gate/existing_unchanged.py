#!/usr/bin/env python3
"""Dynamic parity scenes, C2 gate 1: the runner's EXISTING rows are unchanged, lane exe vs parent exe.

usage: existing_unchanged.py LANE_EXE PARENT_EXE OUTDIR [--only NAME ...]

Runs every row below on both exes (one process each, in OUTDIR/<name>/{lane,parent}.*) and requires:
equal exit codes; equal SUMMARY JSON once the timing keys are removed; equal non-timing CSV columns
(`step,manifolds,pairs,top_y,awake`, armed also `void,colors,wide_colors,waves`). Timing keys:
window_mean_ns, window_steps_per_s, ticks_per_ns, the whole `w8s` object (its means are times and
its route and in-flight sums are scheduling), threads.solve_on_dispatcher_steps and
threads.dispatcher_lane_samples_max; and `args`, which carries each side's own output paths. Prints one line per row and a total; exit 0 iff every row
is equal. No timing is read.
"""
import json
import os
import subprocess
import sys

ROWS = [
    ('J500_W1', ['--scene', 'jolt', '--steps', '500', '--workers', '1']),
    ('J500_W8', ['--scene', 'jolt', '--steps', '500', '--workers', '8']),
    ('J500off_W1', ['--scene', 'jolt', '--steps', '500', '--workers', '1', '--contact-reuse', 'off']),
    ('J500d0_W1', ['--scene', 'jolt', '--steps', '500', '--workers', '1', '--speculative-distance', '0',
                   '--speculative-velocity-cap', '0']),
    ('R1100_W1', ['--scene', 'rest', '--solver', 'colored', '--steps', '1100', '--workers', '1']),
    ('S16a_W1', ['--scene', 's16', '--cfg', 'a', '--steps', '300', '--workers', '1']),
    ('J500sleep_W1', ['--scene', 'jolt', '--steps', '500', '--workers', '1', '--sleeping', 'on']),
    ('J100allpairs_W1', ['--scene', 'jolt', '--steps', '100', '--workers', '1', '--broadphase', 'allpairs']),
    ('J100armed_W1', ['--scene', 'jolt', '--steps', '100', '--workers', '1', '--arm-profiler']),
    ('J100armed_W8', ['--scene', 'jolt', '--steps', '100', '--workers', '8', '--arm-profiler']),
    ('selfcheck', []),
]
# `args` carries each side's own --csv / --pose-out paths; the command is otherwise the same.
TIMING = {'window_mean_ns', 'window_steps_per_s', 'ticks_per_ns', 'w8s', 'args'}
THREAD_TIMING = {'solve_on_dispatcher_steps', 'dispatcher_lane_samples_max'}
CSV_COLS = ['step', 'manifolds', 'pairs', 'top_y', 'awake']
CSV_ARMED = ['void', 'colors', 'wide_colors', 'waves']


def run(exe, args, prefix):
    full = list(args)
    if args:
        full += ['--csv', prefix + '.csv', '--pose-out', prefix + '.pose']
    r = subprocess.run([exe] + full, capture_output=True, text=True)
    with open(prefix + '.out', 'w', encoding='utf-8') as f:
        f.write(r.stdout + r.stderr)
    summaries = [ln[len('SUMMARY '):] for ln in r.stdout.splitlines() if ln.startswith('SUMMARY ')]
    return r.returncode, summaries


def strip(summary):
    d = json.loads(summary)
    for k in TIMING:
        d.pop(k, None)
    for k in THREAD_TIMING:
        d.get('threads', {}).pop(k, None)
    return d


def csv_cols(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        lines = f.read().splitlines()
    head = lines[0].split(',')
    cols = [c for c in CSV_COLS + CSV_ARMED if c in head]
    idx = [head.index(c) for c in cols]
    return cols, [[ln.split(',')[i] for i in idx] for ln in lines[1:]]


def main():
    if len(sys.argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    lane, parent, out = (os.path.abspath(x) for x in sys.argv[1:4])
    only = set(sys.argv[5:]) if len(sys.argv) > 4 and sys.argv[4] == '--only' else None
    bad = 0
    n = 0
    for name, args in ROWS:
        if only and name not in only:
            continue
        n += 1
        d = os.path.join(out, name)
        os.makedirs(d, exist_ok=True)
        rc_l, s_l = run(lane, args, os.path.join(d, 'lane'))
        rc_p, s_p = run(parent, args, os.path.join(d, 'parent'))
        why = []
        if rc_l != rc_p:
            why.append('exit %d vs %d' % (rc_l, rc_p))
        if len(s_l) != 1 or len(s_p) != 1:
            why.append('summaries %d vs %d' % (len(s_l), len(s_p)))
        else:
            a, b = strip(s_l[0]), strip(s_p[0])
            if a != b:
                keys = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
                why.append('summary keys differ: %s' % keys)
        if args:
            ca, cb = csv_cols(os.path.join(d, 'lane.csv')), csv_cols(os.path.join(d, 'parent.csv'))
            if ca != cb:
                why.append('csv columns differ')
            pa, pb = (open(os.path.join(d, x + '.pose'), 'rb').read() if os.path.exists(os.path.join(d, x + '.pose')) else None
                      for x in ('lane', 'parent'))
            if pa != pb:
                why.append('pose bytes differ')
        bad += bool(why)
        h = json.loads(s_l[0]).get('pose_hash') if len(s_l) == 1 else None
        print('%-18s %s exit %d pose %s%s' % (name, 'SAME' if not why else 'DIFF', rc_l, h, ('  ' + '; '.join(why)) if why else ''))
    print('rows %d, same %d, diff %d' % (n, n - bad, bad))
    return 0 if bad == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
