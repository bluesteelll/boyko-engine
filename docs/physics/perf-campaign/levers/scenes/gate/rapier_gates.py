#!/usr/bin/env python3
"""Dynamic parity scenes, C4: the Rapier 0.36 dyn/ arms, structural only (no timing column is read).

usage: rapier_gates.py OUTDIR [--gate 1|2|3|all] [--rows ID,ID,...] [--exe-dir DIR] [--frozen-exe-dir DIR]
                              [--write-fixtures] [--lock PATH] [--sums PATH] [--pins PATH]

G1  the freeze is intact: every file sha256 FROZEN-9b section 1 pins (frozen9b.json `files`), the frozen exes
    against gate/bin/SHA256SUMS(.block), and dyn/Cargo.toml, dyn/Cargo.lock byte-identical to the frozen
    ones and dyn/src/spec.rs to boyko's committed dyn_spec.rs (`git show HEAD:...`, the LF blob).
    `--lock PATH` checks PATH in place of dyn/Cargo.lock (the red-first).
    The dyn sources: every file dyn/bin/SOURCES.sha256 pins (the build's record of its inputs) hashes to its
    pin both live under dyn/ and as the repository's copy (scenes/rapier/*.txt, HEAD blobs; spec.rs as
    dyn_spec.rs). crates/boyko_physics/tests/dyn_scenes_hook_order.rs reads main.rs.txt, so this is what makes
    its verdict about the source the dyn exes were built from (triage r1 F1). `--sums PATH` checks PATH in
    place of SOURCES.sha256 (the red-first).
    The dyn exes: each arm's exe hashes to dyn/bin/SHA256SUMS (the build's record of its outputs) and to
    pins.json `exes` (fix r1 rebuilt them; `--pins` for the red-first).
G2  J-T unchanged by the hooks: all 70 frozen rows' commands (frozen9b.json `command`) on the dyn exe of the
    row's arm at W1 and W8, each exit 0 with `--expect-pose` = the row's fixture; and for one row per arm at
    W1, the SUMMARY equal to the frozen exe's minus the source hashes, `args` and the timing keys.
G3  the dynamic programs on the ruling-22 rows (9b's 7-row R-CLAIM union plus simd8/D0): the dump equals
    the canonical text; determinism (W1 twice, W8 twice); the pose at W1/2/4/8/16 equals W1's; --sanity
    changes no bit; dyn_sanity.py B1-B6 and the premises at W1, S-LAND (J-T --sanity) included; the summary's
    counts and void list; a --receipt run's work counts (rows, Np, Nm over the metric window); the W1 pose
    and its hash against pins.json `rapier.<row>.<program>` (pose_pin.check_pose: the fixture's sha256, then
    the bytes; the one check against the committed pin rather than the run itself, triage r1 F2; `--pins`
    for the red-first); and, with --write-fixtures (after that check), dyn/fixtures/<arm>/<row>_<program>.pose
    and its sha256 into OUTDIR/rapier_pins.json.
Writes OUTDIR/gates.tsv and prints it.

Exit (docs/physics/perf-campaign/GATE-KIT.md): 0 = at least one check ran and none FAILed; 1 = red (a FAIL,
no check at all, or a G2 whose --rows selected no row); 2 = could not run (an argparse error, or an input
the selected gates read is missing: the Rapier tree, frozen9b.json, the dyn or frozen exe directory),
decided before the first check. An uncaught exception stays Python's 1.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys

RP = 'D:/tmp/rapier-parity'
FROZEN = RP + '/sweep/frozen9b.json'
REPO = 'D:/wt/merge'
SPEC_BLOB = 'crates/boyko_physics/benches/jolt_parity_pyramid/dyn_spec.rs'
COPIES = 'docs/physics/perf-campaign/levers/scenes/rapier/'
# The repository's copy of each dyn source, by the name dyn/bin/SOURCES.sha256 gives it. The copies are
# `-text` (.gitattributes), so a HEAD blob is the file's bytes.
SOURCE_COPIES = {'src/main.rs': COPIES + 'main.rs.txt', 'src/dyn_scenes.rs': COPIES + 'dyn_scenes.rs.txt',
                 'src/spec.rs': SPEC_BLOB, 'build_dyn.sh': COPIES + 'build_dyn.sh.txt',
                 '.cargo/config.toml': COPIES + 'cargo_config.toml.txt'}
G = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, G)
import pose_pin  # noqa: E402  (beside this script)
Q1_ROWS = ['simd8/s1p7q2pd0+roff', 'simd8/s1p8q2pd0+roff', 'simd8/s1p9q2', 'simd4/s1p8q3pd0+roff',
           'block/s1p4q3pd0+roff', 'block/s1p5q2pd0+roff', 'block/s1p5q3pd0+roff', 'simd8/D0']
STEPS = {'none': 500, 'kick': 800, 'shoot': 800, 'slide': 500}
WINDOW = {'none': (100, 500), 'kick': (200, 800), 'shoot': (200, 800), 'slide': (0, 500)}
TIMING = {'window_mean_ns', 'window_steps_per_s', 'args', 'source_fnv1a64', 'label'}

T = []


def check(name, ok, detail=''):
    v = ok if isinstance(ok, str) else ('PASS' if ok else 'FAIL')
    T.append('%s\t%s\t%s' % (name, v, detail))
    print('%s\t%s\t%s' % (name, v, detail), flush=True)


def sha256(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def run(exe, args, out_prefix):
    r = subprocess.run([exe] + args, capture_output=True, text=True)
    with open(out_prefix + '.out', 'w', encoding='utf-8') as f:
        f.write(r.stdout + r.stderr)
    s = [ln[8:] for ln in r.stdout.splitlines() if ln.startswith('SUMMARY ')]
    return r.returncode, (json.loads(s[0]) if len(s) == 1 else None)


def row_args(row, w, prefix, fixture=True):
    """The frozen command with W, the paths and the fixture filled in."""
    a = ['--cfg', row['cfg']] + row['flags'] + ['--workers', str(w), '--steps', '500', '--window', '100..500',
                                               '--pose-out', prefix + '.pose']
    if fixture:
        a += ['--expect-pose', os.path.join(RP, row['fixture'])]
    return a


def gate1_sources(sums_path):
    """The dyn sources, live and as the repository's copies, against the build's SOURCES.sha256."""
    pinned = {}
    for ln in open(sums_path or RP + '/dyn/bin/SOURCES.sha256', encoding='utf-8'):
        if ln.strip():
            h, name = ln.split(None, 1)
            pinned[name.strip().lstrip('*')] = h
    bad = ['%s not in SOURCES.sha256' % n for n in sorted(set(SOURCE_COPIES) - set(pinned))]
    for name, want in sorted(pinned.items()):
        live = os.path.join(RP, 'dyn', name)
        if not os.path.exists(live) or sha256(live) != want:
            bad.append('%s live' % name)
        if name in SOURCE_COPIES:
            blob = subprocess.run(['git', '-C', REPO, 'show', 'HEAD:' + SOURCE_COPIES[name]], capture_output=True).stdout
            if hashlib.sha256(blob).hexdigest() != want:
                bad.append('%s repo copy %s' % (name, SOURCE_COPIES[name].rsplit('/', 1)[-1]))
    check('G1 dyn sources = SOURCES.sha256, live and the repo copies (%d files, %d copies)' % (len(pinned), len(SOURCE_COPIES)),
          not bad, ', '.join(bad) or 'all equal')


def gate1_exes(pins_path):
    """The dyn exes against the build's SHA256SUMS and pins.json `exes`."""
    pins = json.load(open(pins_path or pose_pin.PINS))['exes']
    built = {}
    for ln in open(RP + '/dyn/bin/SHA256SUMS', encoding='utf-8'):
        if ln.strip():
            h, name = ln.split(None, 1)
            built[os.path.basename(name.strip().lstrip('*'))] = h
    bad = []
    for arm in ('simd8', 'simd4', 'block'):
        p = pins['rapier-dyn-' + arm]
        live = sha256(p['path']) if os.path.exists(p['path']) else None
        if not (live == p['sha256'] == built.get(os.path.basename(p['path']))):
            bad.append(arm)
    check('G1 dyn exes = dyn/bin/SHA256SUMS = pins.json exes (3 arms)', not bad, ','.join(bad) or 'all equal')


def gate1(out, lock, sums, pins_path):
    gate1_sources(sums)
    gate1_exes(pins_path)
    d = json.load(open(FROZEN))
    bad = []
    for name, want in d['files'].items():
        path = os.path.join(RP, name.split(' ')[0])
        got = sha256(path) if os.path.exists(path) else None
        if got != want:
            bad.append(name.split(' ')[0])
    check('G1 frozen files (%d, frozen9b.json files)' % len(d['files']), not bad, ','.join(bad) or 'all equal')
    r = subprocess.run(['sha256sum', '-c', 'gate/bin/SHA256SUMS', 'gate/bin/SHA256SUMS.block'], cwd=RP, capture_output=True, text=True)
    check('G1 frozen exes (SHA256SUMS, SHA256SUMS.block)', r.returncode == 0, r.stdout.strip().replace('\n', '; '))
    same = lambda a, b: open(a, 'rb').read() == open(b, 'rb').read()
    check('G1 dyn/Cargo.toml = Cargo.toml', same(RP + '/dyn/Cargo.toml', RP + '/Cargo.toml'))
    check('G1 dyn/Cargo.lock = Cargo.lock', same(lock or RP + '/dyn/Cargo.lock', RP + '/Cargo.lock'), lock or '')
    blob = subprocess.run(['git', '-C', REPO, 'show', 'HEAD:' + SPEC_BLOB], capture_output=True).stdout
    check('G1 dyn/src/spec.rs = boyko dyn_spec.rs (HEAD blob)', open(RP + '/dyn/src/spec.rs', 'rb').read() == blob,
          hashlib.sha256(blob).hexdigest()[:16])


def gate2(out, exe_dir, frozen_dir, only):
    d = json.load(open(FROZEN))
    rows = [r for r in d['rows'] if not only or r['id'] in only]
    first = {}
    fails = []
    for r in rows:
        exe = os.path.join(exe_dir, 'rapier-parity-dyn-%s.exe' % r['arm'])
        for w in (1, 8):
            p = os.path.join(out, 'g2', r['id'].replace('/', '__') + '_W%d' % w)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            rc, s = run(exe, row_args(r, w, p), p)
            ok = rc == 0 and s is not None and s.get('expect_pose') == 'match' and s.get('pose_hash') == r['pose_hash']
            if not ok:
                fails.append('%s W%d rc %d %s' % (r['id'], w, rc, s and s.get('expect_pose')))
            if w == 1 and r['arm'] not in first:
                first[r['arm']] = (r, s)
    # A G2 that selected no row checked nothing: that is a FAIL, never a vacuous PASS.
    check('G2 frozen rows on the dyn exes, W1 and W8 (%d rows, %d runs)' % (len(rows), 2 * len(rows)), bool(rows) and not fails,
          '; '.join(fails[:5]) or ('all exit 0, expect_pose match, pose_hash = frozen9b' if rows else 'no rows selected'))
    for arm, (r, s_dyn) in sorted(first.items()):
        p = os.path.join(out, 'g2', r['id'].replace('/', '__') + '_W1_frozen')
        rc, s_fro = run(os.path.join(frozen_dir, 'rapier-parity-%s.exe' % arm), row_args(r, 1, p), p)
        a = {k: v for k, v in (s_dyn or {}).items() if k not in TIMING}
        b = {k: v for k, v in (s_fro or {}).items() if k not in TIMING}
        diff = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
        check('G2 SUMMARY dyn = frozen exe (%s, W1)' % r['id'], rc == 0 and not diff, ','.join(diff) or 'equal')


def gate3(out, exe_dir, only, write_fixtures, pins_path):
    d = json.load(open(FROZEN))
    pinned = json.load(open(pins_path or pose_pin.PINS))['rapier']
    by_id = {r['id']: r for r in d['rows']}
    canon = os.path.join(out, 'canon')
    os.makedirs(canon, exist_ok=True)
    subprocess.run([sys.executable, os.path.join(G, 'dyn_spec_ref.py'), '--write', canon], capture_output=True)
    pins = {}
    for rid in (only or Q1_ROWS):
        r = by_id[rid]
        exe = os.path.join(exe_dir, 'rapier-parity-dyn-%s.exe' % r['arm'])
        base_flags = ['--cfg', r['cfg']] + r['flags']
        # S-LAND: the J-T row with --sanity (the bar on the landing; critique O4)
        p = os.path.join(out, 'g3', rid.replace('/', '__'), 'none_san1')
        os.makedirs(os.path.dirname(p), exist_ok=True)
        rc, s = run(exe, base_flags + ['--workers', '1', '--steps', '500', '--sanity', p + '.csv', '--scene-dump', p + '.dump',
                                       '--pose-out', p + '.pose', '--expect-pose', os.path.join(RP, r['fixture'])], p)
        bar = subprocess.run([sys.executable, os.path.join(G, 'dyn_sanity.py'), '--program', 'none', p + '.csv'], capture_output=True, text=True)
        cmp = subprocess.run([sys.executable, os.path.join(G, 'compare_dumps.py'), p + '.dump', os.path.join(canon, 'none.dump')], capture_output=True, text=True)
        check('G3 %s none: --sanity on the fixture, bar, dump' % rid, rc == 0 and bar.returncode == 0 and cmp.returncode == 0,
              '%s | %s' % (bar.stdout.splitlines()[0] if bar.stdout else bar.stderr.strip(), cmp.stdout.strip()))
        for prog in ('kick', 'shoot', 'slide'):
            dd = os.path.join(out, 'g3', rid.replace('/', '__'), prog)
            os.makedirs(dd, exist_ok=True)
            base = base_flags + ['--dyn', prog]
            res = {}
            for tag, w, extra in [('w1', 1, []), ('w2', 2, []), ('w4', 4, []), ('w8', 8, []), ('w16', 16, []), ('w1b', 1, []), ('w8b', 8, []),
                                  ('san1', 1, ['--sanity', os.path.join(dd, 'sanity_w1.csv'), '--scene-dump', os.path.join(dd, 'dump_w1.txt')]),
                                  ('rcpt1', 1, ['--receipt', '--window', '%d..%d' % WINDOW[prog]])]:
                pp = os.path.join(dd, tag)
                res[tag] = run(exe, base + ['--workers', str(w), '--pose-out', pp + '.pose'] + extra, pp)
            pose = lambda t: open(os.path.join(dd, t + '.pose'), 'rb').read()
            rcs = sorted({v[0] for v in res.values()})
            voids = sorted({v for t in res for v in (res[t][1] or {}).get('voids', ['no summary'])})
            check('G3 %s %s exits and voids' % (rid, prog), rcs == [0] and not voids, 'rc %s voids %s' % (rcs, voids))
            cw = [t for t in ('w2', 'w4', 'w8', 'w16') if pose(t) != pose('w1')]
            check('G3 %s %s pose W1/2/4/8/16' % (rid, prog), not cw, ('differs at ' + ','.join(cw)) if cw else (res['w1'][1] or {}).get('pose_hash'))
            check('G3 %s %s determinism' % (rid, prog), pose('w1') == pose('w1b') and pose('w8') == pose('w8b'))
            check('G3 %s %s --sanity / --receipt change no bit' % (rid, prog), pose('w1') == pose('san1') == pose('rcpt1'))
            c = subprocess.run([sys.executable, os.path.join(G, 'compare_dumps.py'), os.path.join(dd, 'dump_w1.txt'), os.path.join(canon, prog + '.dump')], capture_output=True, text=True)
            check('G3 %s %s dump = canonical' % (rid, prog), c.returncode == 0, c.stdout.strip())
            b = subprocess.run([sys.executable, os.path.join(G, 'dyn_sanity.py'), '--program', prog, os.path.join(dd, 'sanity_w1.csv'), '--json', os.path.join(dd, 'bar_w1.json')], capture_output=True, text=True)
            check('G3 %s %s bar B1-B6 + premise (W1)' % (rid, prog), b.returncode == 0, (b.stdout.splitlines() or [b.stderr])[0])
            dy = (res['san1'][1] or {}).get('dyn', {})
            want = {'kick': (1488, 0, 1240), 'shoot': (0, 60, 1300), 'slide': (0, 0, 1240)}[prog]
            got = (dy.get('kicks'), dy.get('launches'), dy.get('bodies'))
            check('G3 %s %s counts' % (rid, prog), got == want and dy.get('readback_mismatches') == 0 and dy.get('bad_bodies') == 0,
                  'kicks/launches/bodies %s, mismatches %s, bad bodies %s' % (got, dy.get('readback_mismatches'), dy.get('bad_bodies')))
            pin = pinned.get(rid, {}).get(prog)
            if pin is None:
                check('G3 %s %s W1 pose = pin (pins.json)' % (rid, prog), False, 'no pin')
            else:
                ok, det = pose_pin.check_pose(os.path.join(dd, 'w1.pose'), os.path.join(RP, pin['fixture']), pin['fixture_sha256'])
                h = (res['w1'][1] or {}).get('pose_hash')
                check('G3 %s %s W1 pose = pin (pins.json)' % (rid, prog), ok and h == pin['pose_hash'],
                      '%s; hash %s, pinned %s' % (det, h, pin['pose_hash']))
            wm = (res['rcpt1'][1] or {}).get('window_mean') or {}
            pins.setdefault(rid, {})[prog] = {
                'pose_hash': (res['w1'][1] or {}).get('pose_hash'),
                'receipt_window': list(WINDOW[prog]),
                'rows': wm.get('rows'), 'Np': wm.get('points'), 'Nm': wm.get('manifolds'),
                'statics': (dy.get('sanity') or {}).get('statics'),
            }
            if write_fixtures:
                fx = os.path.join(RP, 'dyn', 'fixtures', r['arm'])
                os.makedirs(fx, exist_ok=True)
                dst = os.path.join(fx, '%s_%s.pose' % (r['config'] + ('+roff' if r['twin'] else ''), prog))
                shutil.copyfile(os.path.join(dd, 'w1.pose'), dst)
                pins[rid][prog]['fixture'] = os.path.relpath(dst, RP).replace('\\', '/')
                pins[rid][prog]['fixture_sha256'] = sha256(dst)
    with open(os.path.join(out, 'rapier_pins.json'), 'w') as f:
        json.dump(pins, f, indent=1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out')
    ap.add_argument('--gate', default='all', choices=['1', '2', '3', 'all'])
    ap.add_argument('--rows')
    ap.add_argument('--exe-dir', default=RP + '/dyn/bin')
    ap.add_argument('--frozen-exe-dir', default=RP + '/gate/bin')
    ap.add_argument('--write-fixtures', action='store_true')
    ap.add_argument('--lock')
    ap.add_argument('--sums')
    ap.add_argument('--pins')
    a = ap.parse_args()
    need = [RP, FROZEN]
    if a.gate in ('2', '3', 'all'):
        need.append(a.exe_dir)
    if a.gate in ('2', 'all'):
        need.append(a.frozen_exe_dir)
    missing = [p for p in need if not os.path.exists(p)]
    if missing:
        print('rapier_gates.py: missing input %s' % ', '.join(missing), file=sys.stderr)
        return 2
    os.makedirs(a.out, exist_ok=True)
    only = a.rows.split(',') if a.rows else None
    if a.gate in ('1', 'all'):
        gate1(a.out, a.lock, a.sums, a.pins)
    if a.gate in ('2', 'all'):
        gate2(a.out, a.exe_dir, a.frozen_exe_dir, only)
    if a.gate in ('3', 'all'):
        gate3(a.out, a.exe_dir, only, a.write_fixtures, a.pins)
    with open(os.path.join(a.out, 'gates.tsv'), 'a') as f:
        f.write('\n'.join(T) + '\n')
    n_fail = sum(1 for t in T if '\tFAIL\t' in t)
    print('checks %d, fail %d' % (len(T), n_fail))
    return 0 if n_fail == 0 and T else 1


if __name__ == '__main__':
    sys.exit(main())
