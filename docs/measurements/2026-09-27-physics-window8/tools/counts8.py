"""Untimed validity counts of a window-8 run (raw/runs.jsonl): per block and row, the processes, valid, contaminated
(by cause), re-runs, hangs, not-measured DM1 rows and voided passes. Reads no timing statistic."""
import collections
import json
import os
import sys

raw = sys.argv[1]
p = os.path.join(raw, 'runs.jsonl')
if not os.path.exists(p):
    print('no runs.jsonl')
    sys.exit(0)
recs = [json.loads(l) for l in open(p, encoding='utf-8') if l.strip()]
procs = [r for r in recs if 'row' in r]
print(f'records {len(recs)}: processes {len(procs)}, warm-ups {sum(1 for r in procs if r.get("attempt") == "warmup")}, '
      f'passes done {sum(1 for r in recs if r.get("pass_done"))}, voided passes {sum(1 for r in recs if r.get("voided_pass"))}, '
      f'blocks skipped {[r["block"] for r in recs if r.get("block_skipped")]}')
by = collections.defaultdict(lambda: collections.Counter())
for r in procs:
    if r.get('attempt') == 'warmup':
        continue
    k = (r['block'], r['row'], r['binary'], r['W'])
    c = by[k]
    c['n'] += 1
    c['rerun'] += r.get('attempt') == 'rerun'
    c['valid'] += bool(r.get('valid'))
    c['clean_valid'] += bool(r.get('valid')) and not r.get('contaminated') and not r.get('void')
    for f in ('contaminated_before', 'contaminated_after', 'contaminated_during', 'contaminated_witness', 'hang', 'not_measured', 'void'):
        c[f] += bool(r.get(f))
    c['aborted'] += 'aborted' in r
print(f'{"block":14s} {"row":28s} {"bin":5s} {"W":>3s} {"n":>3s} {"rerun":>5s} {"valid":>5s} {"clean":>5s} before after build witness hang notmeas void')
for (b, row, key, w), c in by.items():
    print(f'{b:14s} {row:28s} {key:5s} {w:3d} {c["n"]:3d} {c["rerun"]:5d} {c["valid"]:5d} {c["clean_valid"]:5d} '
          f'{c["contaminated_before"]:6d} {c["contaminated_after"]:5d} {c["contaminated_during"]:5d} {c["contaminated_witness"]:7d} '
          f'{c["hang"]:4d} {c["not_measured"]:7d} {c["void"]:4d}')
bad = [r for r in procs if r.get('invalid')]
for r in bad[:40]:
    print(f'INVALID {r["block"]} {r["row"]}#{r["binary"]}@W{r["W"]} {r.get("attempt")}: {"; ".join(r["invalid"])}')
