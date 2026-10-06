#!/usr/bin/env python3
"""Dynamic parity scenes: THE sanity bar, one scorer for every engine (ours, Jolt 5.6, Rapier 0.36).

usage:
  dyn_sanity.py --program P CSV [--steps N] [--json OUT] [--old-b4]
      the per-step bar (B1-B6) and the scene's premise checks on a sanity CSV
      (ours `--sanity PATH`, Rapier `--sanity PATH`, Jolt `-sanity` -> sanity_<tag>.csv)
  dyn_sanity.py --final P POSE
      B1-B3 on a final pose file (13 little-endian f32 per dynamic body: p, v, q, w)
exit: 0 pass, 1 fail, 2 usage or unreadable input.

The bar (01-DESIGN.md, "The sanity bar"; constants fixed before any engine ran against them, PC-DYN-8):
  B1  a body state value is non-finite
  B2  a body centre is outside the scene's bounds (y >= 0 everywhere; |x|, |z| <= 50; arena scenes
      also y <= 61, and S-SLIDE x <= 30)
  B3  a body is faster than 70 m/s
  B4  energy created from nothing: with D(s) = e_total(s) - e_inj(s) (the mechanical energy net of
      everything the events injected), D(s) - min over s' < s of D(s') > 620 per unit J-T box mass
      (every J-T box gaining 1 m/s). Physically D never rises (contacts and friction only
      dissipate), so the running minimum makes the bar tight after the pile settles, not only at
      step 0 (critique W1: the cumulative form `e_total(s) <= e_total(0) + e_inj(s) + eps` could not
      fail after the landing had dissipated 20,601).
  B5  a launch's spawn point is closer than 2*sqrt(3) m to a dynamic body's centre
  B6  the engine reported an error for a step (Jolt's EPhysicsUpdateError)
Premises (the scene did what it says): S-KICK every kick event injects energy (24 jumps of e_inj);
S-SHOOT the body count grows by one per launch (60); S-SLIDE the centroid moves >= 5 m downhill
(+x); S-LAND the body count stays 1240.
`--old-b4` also evaluates the cut's original cumulative B4 (eps 1030), for the W1 red-first only.
"""
import argparse
import json
import math
import struct
import sys

MAX_SPEED = 70.0
EPS = 620.0
OLD_EPS = 1030.0
CLEARANCE = 2.0 * math.sqrt(3.0)
JT = 1240
STEPS = {'none': 500, 'kick': 800, 'shoot': 800, 'slide': 500}
WINDOW = {'none': (0, 100), 'kick': (200, 800), 'shoot': (200, 800), 'slide': (0, 500)}
INF = float('inf')
BOUNDS = {
    'none': ((-50.0, 0.0, -50.0), (50.0, INF, 50.0)),
    'kick': ((-50.0, 0.0, -50.0), (50.0, INF, 50.0)),
    'shoot': ((-50.0, 0.0, -50.0), (50.0, 61.0, 50.0)),
    'slide': ((-50.0, 0.0, -50.0), (30.0, 61.0, 50.0)),
}
KICK_STEPS = [200 + 25 * k for k in range(24)]
LAUNCH_STEPS = [200 + 10 * k for k in range(60)]
COLUMNS = ['step', 'e_lin', 'e_rot', 'e_pot', 'e_total', 'e_inj', 'max_speed', 'min_x', 'max_x', 'min_y',
           'max_y', 'min_z', 'max_z', 'cx', 'cy', 'cz', 'nonfinite', 'bodies', 'clearance', 'engine_err']


def num(s):
    return float(s.strip())


def load_csv(path):
    with open(path, encoding='ascii') as f:
        lines = [ln.rstrip('\r\n') for ln in f if ln.strip()]
    header = [c.strip() for c in lines[0].split(',')]
    if header != COLUMNS:
        raise ValueError('header %r is not the sanity schema %r' % (header, COLUMNS))
    rows = []
    for ln in lines[1:]:
        cells = ln.split(',')
        if len(cells) != len(COLUMNS):
            raise ValueError('row %r has %d cells' % (ln, len(cells)))
        r = {}
        for k, c in zip(COLUMNS, cells):
            c = c.strip()
            if k == 'clearance':
                r[k] = num(c) if c else None
            elif k in ('step', 'nonfinite', 'bodies', 'engine_err'):
                r[k] = int(c)
            else:
                r[k] = num(c)
        rows.append(r)
    return rows


def score_csv(program, rows, steps, old_b4=False):
    fails = []
    premise = []

    def fail(bar, s, why):
        fails.append({'bar': bar, 'row': s, 'why': why})

    if [r['step'] for r in rows] != list(range(steps + 1)):
        fail('S', None, 'rows are not the states 0..%d (got %d rows)' % (steps, len(rows)))
    lo, hi = BOUNDS[program]
    d_min = INF
    rise_max = -INF
    for r in rows:
        s = r['step']
        if r['nonfinite'] > 0 or any(not math.isfinite(r[k]) for k in ('e_total', 'e_inj', 'max_speed')):
            fail('B1', s, '%d bodies with a non-finite state value' % r['nonfinite'])
        for a, ax in enumerate('xyz'):
            if r['min_' + ax] < lo[a] or r['max_' + ax] > hi[a]:
                fail('B2', s, '%s in [%r, %r], bounds [%r, %r]' % (ax, r['min_' + ax], r['max_' + ax], lo[a], hi[a]))
        if r['max_speed'] > MAX_SPEED:
            fail('B3', s, 'speed %r m/s' % r['max_speed'])
        d = r['e_total'] - r['e_inj']
        if s > 0:
            rise = d - d_min
            rise_max = max(rise_max, rise)
            if rise > EPS:
                fail('B4', s, 'D rose %r above its running minimum %r (eps %r)' % (rise, d_min, EPS))
        d_min = min(d_min, d)
        if r['clearance'] is not None and r['clearance'] < CLEARANCE:
            fail('B5', s, 'launch clearance %r m' % r['clearance'])
        if r['engine_err'] != 0:
            fail('B6', s, 'engine error flags %d' % r['engine_err'])
        if old_b4 and r['e_total'] > rows[0]['e_total'] + r['e_inj'] + OLD_EPS:
            fail('B4old', s, 'cumulative form')
    by = {r['step']: r for r in rows}
    full = steps == STEPS[program]
    if program == 'kick' and full:
        jumps = [t for t in KICK_STEPS if t + 1 in by and by[t + 1]['e_inj'] > by[t]['e_inj']]
        if len(jumps) != 24:
            premise.append('S-KICK: %d of 24 kick events injected energy' % len(jumps))
    if program == 'shoot' and full:
        for r in rows:
            want = JT + sum(1 for t in LAUNCH_STEPS if t < r['step'])
            if r['bodies'] != want:
                premise.append('S-SHOOT: row %d has %d bodies, want %d' % (r['step'], r['bodies'], want))
                break
    if program == 'slide' and full and rows:
        moved = rows[-1]['cx'] - rows[0]['cx']
        if moved < 5.0:
            premise.append('S-SLIDE: the centroid moved %r m downhill, want >= 5' % moved)
    if program in ('none', 'kick', 'slide') and any(r['bodies'] != JT for r in rows):
        premise.append('%s: the body count is not 1240 throughout' % program)
    ke = lambda r: r['e_lin'] + r['e_rot']
    receipts = {
        'rows': len(rows),
        'e0': rows[0]['e_total'] if rows else None,
        'e_inj': rows[-1]['e_inj'] if rows else None,
        'd_rise_max': rise_max if rise_max > -INF else None,
        'max_speed': max(r['max_speed'] for r in rows) if rows else None,
        'min_y': min(r['min_y'] for r in rows) if rows else None,
        'clearance_min': min((r['clearance'] for r in rows if r['clearance'] is not None), default=None),
        'ke_settled_200': ke(by[200]) if 200 in by and program in ('kick', 'shoot') else None,
        'ke_after_event': [[t, ke(by[t + 1])] for t in (KICK_STEPS if program == 'kick' else LAUNCH_STEPS if program == 'shoot' else []) if t + 1 in by],
        'centroid_x': [rows[0]['cx'], rows[-1]['cx']] if rows else None,
    }
    return fails, premise, receipts


def score_pose(program, path):
    data = open(path, 'rb').read()
    if len(data) % 52:
        raise ValueError('%s: %d bytes is not a whole number of 52-byte bodies' % (path, len(data)))
    lo, hi = BOUNDS[program]
    fails = []
    n = len(data) // 52
    for i in range(n):
        f = struct.unpack_from('<13f', data, 52 * i)
        p, v = f[0:3], f[3:6]
        if not all(math.isfinite(x) for x in f):
            fails.append({'bar': 'B1', 'body': i, 'why': 'non-finite state'})
            continue
        for a in range(3):
            if p[a] < lo[a] or p[a] > hi[a]:
                fails.append({'bar': 'B2', 'body': i, 'why': 'centre %r outside the bounds' % (p,)})
                break
        sp = math.sqrt(sum(x * x for x in v))
        if sp > MAX_SPEED:
            fails.append({'bar': 'B3', 'body': i, 'why': 'speed %r' % sp})
    return fails, n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--program', choices=sorted(STEPS))
    ap.add_argument('--final', choices=sorted(STEPS))
    ap.add_argument('--steps', type=int)
    ap.add_argument('--json')
    ap.add_argument('--old-b4', action='store_true')
    ap.add_argument('path')
    a = ap.parse_args()
    if bool(a.program) == bool(a.final):
        print('give exactly one of --program and --final', file=sys.stderr)
        return 2
    try:
        if a.final:
            fails, n = score_pose(a.final, a.path)
            out = {'mode': 'final', 'program': a.final, 'bodies': n, 'fails': fails[:20], 'n_fails': len(fails)}
            premise = []
        else:
            steps = a.steps if a.steps is not None else STEPS[a.program]
            rows = load_csv(a.path)
            fails, premise, receipts = score_csv(a.program, rows, steps, a.old_b4)
            out = {'mode': 'csv', 'program': a.program, 'steps': steps, 'fails': fails[:20], 'n_fails': len(fails),
                   'premise': premise, 'receipts': receipts}
    except (OSError, ValueError) as e:
        print('dyn_sanity: %s' % e, file=sys.stderr)
        return 2
    ok = not fails and not premise
    out['verdict'] = 'PASS' if ok else 'FAIL'
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(out, f, indent=1)
    first = fails[0] if fails else None
    print('%s %s %s%s%s' % (out['verdict'], out['mode'], out['program'],
                            (' first %s at %s: %s' % (first['bar'], first.get('row', first.get('body')), first['why'])) if first else '',
                            (' premise: ' + '; '.join(premise)) if premise else ''))
    if not a.final:
        r = out['receipts']
        print('  receipts: e0 %s e_inj %s d_rise_max %s max_speed %s min_y %s clearance_min %s ke_settled_200 %s centroid_x %s' % (
            r['e0'], r['e_inj'], r['d_rise_max'], r['max_speed'], r['min_y'], r['clearance_min'], r['ke_settled_200'], r['centroid_x']))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
