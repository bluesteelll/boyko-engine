#!/usr/bin/env python3
"""Dynamic parity scenes (triage r1 F2): one run's final pose against the committed pin.

usage: pose_pin.py <ours|jolt56> <program> <pose file> [--hash 0x...] [--pins PATH]
  1. the fixture that pins.json names for (engine, program) hashes to its `fixture_sha256`;
  2. the pose file equals that fixture byte for byte;
  3. with --hash: the pose hash the run printed equals pins.json's (`pose_hash` for ours, the stat
     line's `hash` for Jolt 5.6).
Prints one line, PASS or FAIL with every failed item. Exits 0 on PASS, 1 on FAIL.

Why it exists: every other structural check compares a run with ITSELF (W against W1, a second run
against the first, --sanity against plain). A pose that moved at every W at once passed them all;
tester r1's mutant 7 (one slip used for both the kick's write and its read-back) did exactly that.
Before this check only the window-9c protocol compared a run with the pin. `rapier_gates.py` G3
uses `check_pose` for the Rapier rows' pins.
"""
import argparse
import hashlib
import json
import os
import sys

G = os.path.dirname(os.path.abspath(__file__))
# gate -> scenes -> levers -> perf-campaign -> physics -> docs -> the repository root
REPO = os.path.normpath(os.path.join(G, *['..'] * 6))
PINS = os.path.join(G, '..', 'pins.json')
HASH_KEY = {'ours': 'pose_hash', 'jolt56': 'hash'}


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def check_pose(pose, fixture, fixture_sha256):
    """(ok, detail): the fixture is the pinned bytes, and `pose` equals it byte for byte."""
    bad = []
    if not os.path.isfile(fixture):
        return False, 'no fixture at %s' % fixture
    got = sha256_of(fixture)
    if got != fixture_sha256:
        bad.append('fixture sha256 %s, pinned %s' % (got[:16], fixture_sha256[:16]))
    if not os.path.isfile(pose):
        bad.append('no pose at %s' % pose)
    elif open(pose, 'rb').read() != open(fixture, 'rb').read():
        bad.append('pose differs from %s' % os.path.basename(fixture))
    return not bad, '; '.join(bad) or 'pose = %s (sha256 %s)' % (os.path.basename(fixture), fixture_sha256[:16])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('engine', choices=sorted(HASH_KEY))
    ap.add_argument('program', choices=['none', 'kick', 'shoot', 'slide'])
    ap.add_argument('pose')
    ap.add_argument('--hash')
    ap.add_argument('--pins', default=PINS)
    a = ap.parse_args()
    pin = json.load(open(a.pins))[a.engine][a.program]
    ok, detail = check_pose(a.pose, os.path.join(REPO, pin['fixture']), pin['fixture_sha256'])
    if a.hash is not None and a.hash != pin[HASH_KEY[a.engine]]:
        ok = False
        detail += '; hash %s, pinned %s' % (a.hash or '(none printed)', pin[HASH_KEY[a.engine]])
    elif a.hash is not None:
        detail += '; hash %s' % a.hash
    print('%s %s %s: %s' % ('PASS' if ok else 'FAIL', a.engine, a.program, detail))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
