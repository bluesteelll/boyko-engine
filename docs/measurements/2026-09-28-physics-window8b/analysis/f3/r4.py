"""R4 (tree-f3/window_cmds.md): every process's pose_hash equals its row's hash; per (row pair, W, round) the
leaflist-kd pose.bin equals the leaflist one. Recomputed from the files; the driver's r4 records compared after."""
import itertools
import json
import os
import sys

sys.dont_write_bytecode = True
import lib_f3 as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


recs = L.load_recs()
procs = [r for r in recs if r.get('block') == 'F3' and 'row' in r and not r.get('r4')]
P('processes read: %d (warm-ups %d, originals %d, re-runs %d)' % (
    len(procs), sum(1 for r in procs if r['attempt'] == 'warmup'), sum(1 for r in procs if r['attempt'] == 'original'),
    sum(1 for r in procs if r['attempt'] == 'rerun')))
bad_hash = []
blobs = {}
for r in procs:
    fam = L.row_family(r['row'])
    ss = L.summaries_of(L.read_text(os.path.join(r['cwd'], 'stdout.txt')))
    h = ss[0].get('pose_hash') if ss else None
    if h != L.ROW_HASH[fam]:
        bad_hash.append((r['row'], r['W'], r['pass'], r['round'], r['attempt'], h))
    pb = os.path.join(r['cwd'], 'pose.bin')
    b = open(pb, 'rb').read() if os.path.exists(pb) else None
    r['_pose'] = b
    blobs.setdefault(L.ROW_HASH[fam], set()).add(b)
P('pose_hash != the row hash: %d %s' % (len(bad_hash), bad_hash[:5]))
for h, s in blobs.items():
    P('distinct pose.bin byte strings among processes whose row hash is %s: %d (len %s)' % (
        h, len(s), sorted({len(x) if x else None for x in s})))
fix = os.path.join(L.W8B, 'gate', 'fixtures')
for name, h in (('JT500', '0x30c5438bc6ad9ffa'), ('JTA500', '0x30c5438bc6ad9ffa'), ('RT500', '0x6cbe24bf8fafda26')):
    fb = open(os.path.join(fix, name + '.pose'), 'rb').read()
    P('fixture %s.pose equals the processes\' single pose byte string: %s' % (name, blobs[h] == {fb}))
# per (pair, W, pass, round): every combination of the attempts present
pairs = {}
for r in procs:
    if r['attempt'] == 'warmup':
        continue
    row = L.ROWS[r['row']]
    base = row.get('r4_twin') or r['row']
    if not (row.get('r4_twin') or any(x.get('r4_twin') == r['row'] for x in L.ROWS.values())):
        continue
    pairs.setdefault((base, r['W'], r['pass'], r['round']), {}).setdefault(row['bp_kernel'], []).append(r)
n_keys = n_cmp = n_eq = 0
differ = []
for k, d in sorted(pairs.items()):
    n_keys += 1
    for a, b in itertools.product(d.get('leaflist', []), d.get('leaflist-kd', [])):
        n_cmp += 1
        if a['_pose'] is not None and a['_pose'] == b['_pose']:
            n_eq += 1
        else:
            differ.append((k, a['attempt'], b['attempt']))
    if not d.get('leaflist') or not d.get('leaflist-kd'):
        differ.append((k, 'missing', sorted(d)))
P('(pair, W, pass, round) keys: %d; leaflist x leaflist-kd pose comparisons (every attempt pairing): %d; equal %d; '
  'differ/missing %d %s' % (n_keys, n_cmp, n_eq, len(differ), differ[:5]))
r4 = [r for r in recs if r.get('r4') and r.get('block') == 'F3']
P('driver r4 records (compared): %d, verdicts %s' % (len(r4), sorted({r['verdict'] for r in r4})))
P('R4: %s' % ('HOLDS' if not bad_hash and not differ and all(len(s) == 1 for s in blobs.values()) else 'FAILS'))
open(os.path.join(L.HERE, 'r4.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
