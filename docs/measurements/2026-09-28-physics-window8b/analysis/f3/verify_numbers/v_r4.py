"""R4 recomputed from the pose files and SUMMARY lines (independent)."""
import collections
import hashlib
import itertools
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

recs = L.load_recs()
f3 = [r for r in recs if r.get('block') == 'F3' and r.get('row')]
FIX = {'J': os.path.join(L.W8B, 'gate', 'fixtures', 'JT500.pose'), 'JA': os.path.join(L.W8B, 'gate', 'fixtures', 'JTA500.pose'),
       'R': os.path.join(L.W8B, 'gate', 'fixtures', 'RT500.pose')}
fixb = {k: open(v, 'rb').read() for k, v in FIX.items()}
print('fixture sha256:', {k: hashlib.sha256(v).hexdigest()[:16] for k, v in fixb.items()}, 'J==JA bytes:', fixb['J'] == fixb['JA'])
hash_to_bytes = collections.defaultdict(set)
mism_hash = 0
fix_mism = 0
poses = {}
for r in f3:
    s = L.summary(r['cwd'])
    b = open(os.path.join(r['cwd'], 'pose.bin'), 'rb').read()
    h = s.get('pose_hash')
    hash_to_bytes[h].add(hashlib.sha256(b).hexdigest())
    if h != L.ROWHASH[r['row']]:
        mism_hash += 1
        print('  HASH MISMATCH', r['row'], r['W'], r['pass'], r['round'], r['attempt'], h)
    fk = 'R' if r['row'].startswith('F3-RT') else ('JA' if '-TA-' in r['row'] else 'J')
    if b != fixb[fk]:
        fix_mism += 1
        print('  FIXTURE MISMATCH', r['row'], r['W'], r['pass'], r['round'], r['attempt'])
    poses[(r['row'], r['W'], r['pass'], r['round'], r['attempt'])] = b
print('processes checked (incl. warm-ups, re-runs):', len(f3), 'hash mismatches:', mism_hash, 'fixture byte mismatches:', fix_mism)
print('distinct pose files per summary hash:', {h: len(v) for h, v in hash_to_bytes.items()})

pairs = [('F3-TD-armed-leaflist', 'F3-TD-armed-kd'), ('F3-TA-armed-leaflist', 'F3-TA-armed-kd'),
         ('F3-JT-leaflist', 'F3-JT-kd'), ('F3-RT-leaflist', 'F3-RT-kd')]
keys = 0
pairings = 0
unequal = 0
missing = 0
for ll, kd in pairs:
    Ws = sorted({r['W'] for r in f3 if r['row'] == kd})
    for W in Ws:
        for ps in (0, 1, 2):
            for rd in (0, 1, 2):
                a = [v for (row, w, p, q, at), v in poses.items() if row == ll and w == W and p == ps and q == rd]
                b = [v for (row, w, p, q, at), v in poses.items() if row == kd and w == W and p == ps and q == rd]
                keys += 1
                if not a or not b:
                    missing += 1
                for x, y in itertools.product(a, b):
                    pairings += 1
                    unequal += x != y
print('(pair, W, pass, round) keys:', keys, 'missing:', missing, 'attempt pairings:', pairings, 'unequal:', unequal)
drv = [r for r in recs if r.get('r4')]
print('driver R4 records:', len(drv), 'verdicts:', collections.Counter(r['verdict'] for r in drv))
