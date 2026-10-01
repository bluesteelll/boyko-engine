"""Untimed structure of a --test rehearsal of the block C4-CGU (no timing value is read): process counts, invalid records, the EXECUTED
order and its adjacency (tip and tipcgu1 consecutive at every W of every round), and that each binary key ran ITS OWN exe (path + sha256).
  python -B tools/rehearsal_summary_cgu9a.py <raw dir> [<block>]"""
import json
import os
import sys

d = sys.argv[1]
block = sys.argv[2] if len(sys.argv) > 2 else 'C4-CGU'
recs = [json.loads(l) for l in open(os.path.join(d, 'runs.jsonl'), encoding='utf-8') if l.strip()]
procs = [r for r in recs if 'row' in r and r.get('block') == block and r.get('attempt') != 'warmup']
warm = [r for r in recs if 'row' in r and r.get('block') == block and r.get('attempt') == 'warmup']
print(f'{d}: records {len(recs)}, timed processes {len(procs)}, warm-ups {[(r["row"], r["binary"], r["W"]) for r in warm]}, '
      f'invalid {sum(1 for r in procs + warm if r.get("invalid"))}, non-zero exits {sum(1 for r in procs + warm if r.get("exit") != 0)}, '
      f'passcells {sum(1 for r in recs if r.get("passcell"))}, pass_done {sum(1 for r in recs if r.get("pass_done"))}')
bad = 0
for rnd in sorted({r['round'] for r in procs}):
    seq = sorted((r for r in procs if r['round'] == rnd and r['attempt'] == 'original'), key=lambda r: r['seq'])
    pos = {}
    for i, r in enumerate(seq):
        pos.setdefault(r['W'], []).append((i, r['binary']))
    ok = all(len(v) == 2 and abs(v[0][0] - v[1][0]) == 1 and {b for _, b in v} == {'tip', 'tipcgu1'} for v in pos.values())
    bad += not ok
    print(f'  round {rnd}: {len(seq)} cells in executed order, tip and tipcgu1 adjacent at every W: {ok}; order {[(r["W"], r["binary"]) for r in seq]}')
by = {}
for r in procs + warm:
    by.setdefault(r['binary'], set()).add((os.path.basename(r['exe']), r['exe_sha256']))
print('  exe per binary key:', {k: sorted(v) for k, v in sorted(by.items())})
sys.exit(1 if bad else 0)
