"""Mutation proof for the C4-CGU block and its guards (2026-10-01). Two kinds of mutation, every one must turn the selftest RED:
  D  a textual mutation of tools/window9a_run.py (the text must occur exactly once) -> tools/_mut_cgu.py, then
        python -B tools/selftest9a.py [--no-e2e] --driver tools/_mut_cgu.py        (every case, the pure ones too, sees the mutated copy)
  O  a mutation of the OVERLAY rows9a.extra.json (a copy under logs/_mut_cgu_overlay.json, read by the real driver through W9A_EXTRA, which the driver
     honours under --dry-run, which is how the selftest loads it), then  python -B tools/selftest9a.py --no-e2e
A mutation is CAUGHT when the selftest exits non-zero; the FAIL lines it produced are printed (first three). The unmutated driver/overlay must be green
(the control line). The copies are removed after.
  python -B logs/mutate_cgu.py > logs/mutate_cgu.out"""
import copy
import json
import os
import subprocess
import sys

W9 = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(W9, 'tools', 'window9a_run.py')
MUT = os.path.join(W9, 'tools', '_mut_cgu.py')
OVL = os.path.join(W9, 'logs', '_mut_cgu_overlay.json')
src = open(SRC, encoding='utf-8').read()
real = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
base = json.load(open(os.path.join(W9, 'rows9a.json'), encoding='utf-8'))
TIP_SHA = base['binaries']['tip']['sha256_pin']
TIP_EXE = base['binaries']['tip']['exe']

# (name, old text, new text, needs the end-to-end cases)
D = [
    ('D1 verify_binaries never compares the sha256 pin', "        if e.get('sha256_pin') and got != e['sha256_pin']:", "        if False:", False),
    ('D2 main() does not verify the binaries at start (the wiring case runs processes)',
     "    bad = verify_binaries()\n    if bad:\n        return finish(3, f'STOP at start: binaries do not match bin/SHA256SUMS {bad}', state, None, {'timed': 0})",
     "    bad = []\n    if bad:\n        return finish(3, f'STOP at start: binaries do not match bin/SHA256SUMS {bad}', state, None, {'timed': 0})", True),
    ('D3 cell_order groups the cells by binary (tip and tipcgu1 no longer adjacent)', "                out.append((rid, key, w))\n    return out",
     "                out.append((rid, key, w))\n    out.sort(key=lambda c: c[1])\n    return out", False),
    ('D4 pass_order never reverses', "    return order0[::-1] if p % 2 == 0 else order0", "    return order0", False),
    ('D5 warmups_of keeps only the first warm-up (one exe is never warmed)', "        return [tuple(x) for x in block['warmups']]", "        return [tuple(block['warmups'][0])]", False),
    ('D6 the overlay binaries are not merged (tipcgu1 vanishes)', "        rj['binaries'].update(ex.get('binaries', {}))", "        pass", False),
    ('D7 blocks are not sorted by priority (C4-CGU stays where the overlay put it)', "    rj['blocks'].sort(key=lambda b: b['priority'])", "    pass", False),
    ('D8 the binary\'s config receipt is not checked (a relabelled parent exe passes)', "        if cfg.get(k) != v:\n            why.append(f'config.{k}", "        if False:\n            why.append(f'config.{k}", False),
    ('D9 the pose hash is not compared (a pose change under cgu1 passes)', "        if s.get('pose_hash') != want:", "        if False:", False),
    ('D10 block_estimate prices no cell (the block costs nothing)', "    per_round = sum(est_s(rid, key, w) + RECEIPT_S for rid, key, w in cells)", "    per_round = 0.0", False),
    ('D11 run_one records the WRONG exe for every binary (the first binary\'s)', "    rec.update({'exe': b['path'], 'exe_sha256': b['sha256'],", "    rec.update({'exe': BINS['tip']['path'], 'exe_sha256': BINS['tip']['sha256'],", True),
]


def overlay(edit):
    o = copy.deepcopy(real)
    edit(o)
    return o


def cgu_row(o):
    return next(r for r in o['rows'] if r['id'] == 'C4-JDcgu')


def cgu_blk(o):
    return next(b for b in o['blocks'] if b['name'] == 'C4-CGU')


O = [
    ('O1 tipcgu1 sha256_pin := the tip\'s pin', lambda o: o['binaries']['tipcgu1'].update(sha256_pin=TIP_SHA)),
    ('O2 tipcgu1 exe := the tip exe path (the cgu1 key runs the cgu16 exe)', lambda o: o['binaries']['tipcgu1'].update(exe=TIP_EXE)),
    ('O3 C4-JDcgu args: --sleeping on', lambda o: cgu_row(o).update(args=cgu_row(o)['args'][:-1] + ['on'])),
    ('O4 C4-CGU priority 2.5 (no longer first)', lambda o: cgu_blk(o).update(priority=2.5)),
    ('O5 C4-CGU passes 2 (K 6)', lambda o: cgu_blk(o).update(passes=2)),
    ('O6 C4-CGU warm-ups: only tip', lambda o: cgu_blk(o).update(warmups=[['C4-JDcgu', 'tip', 8]])),
    ('O7 C4-JDcgu metric windows [0,100) [100,500) (not the orchestrator\'s)', lambda o: cgu_row(o).update(metric_windows=[[0, 100], [100, 500]])),
    ('O8 C4-JDcgu pose_ref JA500', lambda o: cgu_row(o).update(pose_ref='JA500')),
    ('O9 the pre-registered claim removed from the block note', lambda o: cgu_blk(o).update(note='K 9 = 3 passes x 3 rounds.')),
    ('O10 C4-JDcgu binaries [tip] only (an A/A-less block)', lambda o: cgu_row(o).update(binaries=['tip'])),
    ('O11 tipcgu1 expect_config removed', lambda o: o['binaries']['tipcgu1'].pop('expect_config')),
    ('O12 C4-JDcgu workers [1,8,16]', lambda o: cgu_row(o).update(workers=[1, 8, 16])),
    ('O13 C4-JDcgu steps 400', lambda o: cgu_row(o).update(steps=400)),
    ('O14 C4-JDcgu tree_diag_expect dropped to the default', lambda o: cgu_row(o).update(tree_diag_expect=None)),
]


def run_selftest(args, env_extra=None):
    e = dict(os.environ)
    e.pop('W9A_EXTRA', None)
    e.update(env_extra or {})
    p = subprocess.run([sys.executable, '-B', os.path.join(W9, 'tools', 'selftest9a.py')] + args, capture_output=True, text=True, cwd=W9, env=e, timeout=3000)
    fails = [l for l in p.stdout.splitlines() if l.startswith('FAIL')]
    last = [l for l in p.stdout.splitlines() if l.startswith('selftest9a:')]
    return p.returncode, fails, (last[0] if last else p.stderr[-200:])


def main():
    caught = 0
    total = 0
    rc, fails, last = run_selftest(['--no-e2e'])
    print(f'CONTROL (unmutated driver and overlay, --no-e2e): exit {rc}, {last}')
    assert rc == 0, 'the control must be green'
    try:
        for name, old, new, needs_e2e in D:
            assert src.count(old) == 1, f'{name}: {src.count(old)} matches'
            open(MUT, 'w', encoding='utf-8', newline='\n').write(src.replace(old, new))
            rc, fails, last = run_selftest((['--driver', MUT] if needs_e2e else ['--no-e2e', '--driver', MUT]))
            total += 1
            caught += rc != 0
            print(f'{"CAUGHT" if rc else "MISSED"} {name}: exit {rc}, {len(fails)} FAIL: ' + ' || '.join(f[:150] for f in fails[:3]) + ('' if rc else f' [{last}]'), flush=True)
        for name, edit in O:
            json.dump(overlay(edit), open(OVL, 'w', encoding='utf-8', newline='\n'), indent=1)
            rc, fails, last = run_selftest(['--no-e2e'], {'W9A_EXTRA': OVL})
            total += 1
            caught += rc != 0
            print(f'{"CAUGHT" if rc else "MISSED"} {name}: exit {rc}, {len(fails)} FAIL: ' + ' || '.join(f[:150] for f in fails[:3]) + ('' if rc else f' [{last}]'), flush=True)
    finally:
        for p in (MUT, OVL):
            if os.path.exists(p):
                os.remove(p)
    print(f'mutations caught: {caught} of {total}')
    return 0 if caught == total else 1


if __name__ == '__main__':
    sys.exit(main())
