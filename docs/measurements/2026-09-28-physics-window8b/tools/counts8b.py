"""Window 8b: + R4 pose-compare records and placement-receipt presence (no value of either is printed).
Untimed validity counts of a window-8 run (raw/runs.jsonl): per block and row, the processes, valid, contaminated
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
procs = [r for r in recs if 'row' in r and not r.get('r4')]  # R4 records carry r4_* keys only; belt and braces
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
r4 = [r for r in recs if r.get('r4')]
print(f'R4 pose compares: {len(r4)}, equal {sum(1 for r in r4 if r["verdict"] == "equal")}, '
      f'DIFFER {sum(1 for r in r4 if r["verdict"] == "DIFFER")}, missing {sum(1 for r in r4 if r["verdict"] == "missing")}')
for r in r4:
    if r['verdict'] != 'equal':
        print(f'R4 {r["verdict"]} {r["block"]}-p{r["pass"]} r{r["round"]} {r["r4_row"]} vs {r["r4_twin"]} W{r["r4_W"]} {r["r4_attempts"]}')
launched = [r for r in procs if 'exit' in r]
print(f'placement receipts: {sum(1 for r in launched if (r.get("placement") or {}).get("main_cycle_frac") is not None)} of '
      f'{len(launched)} launched processes carry the main-thread cycle fraction, '
      f'{sum(1 for r in launched if (r.get("placement") or {}).get("main_share_top_est") is not None)} the top-CPU share estimate; '
      f'{sum(1 for r in launched if (r.get("placement") or {}).get("top3"))} carry the per-CPU ranking')
# 2026-09-29, omega_b v2 (micro8b.py): the per-cell park rule over every rep (the cell's process records, warm-ups
# excluded). A gap > 0 park row with parks_per_region_median 0 on every rep VOIDS that cell; a gap-0 park row with no
# park on any rep is reported "no park taken" (not void). Counts only; no timing value is printed.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import micro8b  # noqa: E402
mrecs = [r for r in procs if r.get('kind') == 'micro' and r.get('attempt') != 'warmup' and r.get('micro_notes')]
if mrecs:
    mvoid, mnopark, mseen = micro8b.cell_park_rule(mrecs)
    print(f'omega_b v2 park rule: {len(mseen)} (row, park config) cells over {len(mrecs)} processes; '
          f'VOID {len(mvoid)}, no park taken (gap 0, reported) {len(mnopark)}')
    for rid, key, n in mvoid:
        print(f'VOID omega_b v2 {rid} {key}: parks_per_region_median 0 on every rep ({n} reps)')
    for rid, key, n in mnopark:
        print(f'no park taken {rid} {key} ({n} reps; gap 0: reported, not void)')
