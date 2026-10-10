"""Untimed structure of a --test rehearsal of a block: process counts, invalid records, the EXECUTED order and its adjacency (no timing value).
  python -B tools/rehearsal_summary9a.py <raw dir> [<block>]"""
import json
import os
import sys

d = sys.argv[1]
block = sys.argv[2] if len(sys.argv) > 2 else 'C4-RAPIER'
recs = [json.loads(l) for l in open(os.path.join(d, 'runs.jsonl'), encoding='utf-8') if l.strip()]
procs = [r for r in recs if 'row' in r and r.get('block') == block and r.get('attempt') != 'warmup']
warm = [r for r in recs if 'row' in r and r.get('block') == block and r.get('attempt') == 'warmup']
print(f'{d}: records {len(recs)}, timed processes {len(procs)}, warm-ups {[(r["row"], r["binary"], r["W"]) for r in warm]}, '
      f'invalid {sum(1 for r in procs + warm if r.get("invalid"))}, non-zero exits {sum(1 for r in procs + warm if r.get("exit") != 0)}, '
      f'passcells {sum(1 for r in recs if r.get("passcell"))}, pass_done {sum(1 for r in recs if r.get("pass_done"))}')
for rnd in sorted({r['round'] for r in procs}):
    seq = sorted((r for r in procs if r['round'] == rnd and r['attempt'] == 'original'), key=lambda r: r['seq'])
    pos = {}
    for i, r in enumerate(seq):
        pos.setdefault(r['W'], []).append(i)
    ok = all(len(v) == 4 and max(v) - min(v) == 3 for v in pos.values())
    print(f'  round {rnd}: {len(seq)} cells in executed order, the four configurations adjacent at every W: {ok}; W groups in order {[r["W"] for r in seq][::4]}')
r0 = procs[0]
print(f'  first process: args {" ".join(r0["args"][:4])} ... --workers {r0["args"][r0["args"].index("--workers") + 1]}, exe {os.path.basename(r0["exe"])}, sha256 {r0["exe_sha256"][:12]}')
print(f'  exes used: {sorted({os.path.basename(r["exe"]) for r in procs + warm})}; receipts present on {sum(1 for r in procs if r.get("placement"))} processes')
