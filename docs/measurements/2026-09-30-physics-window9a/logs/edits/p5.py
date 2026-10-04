import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
apply('tools/counts9a.py', [
 ('"""Window 8b: + R4 pose-compare records and placement-receipt presence (no value of either is printed).',
  '"""Window 9a: + the pass-cell census of ruling 8 (passcell records: k_clean / k_target per (pass, cell); a cell with k_clean < 3\ndoes not gate) and the re-run depth (rerun_no). No timing value is printed.\nWindow 8b: + R4 pose-compare records and placement-receipt presence (no value of either is printed).'),
 ("""procs = [r for r in recs if 'row' in r and not r.get('r4')]  # R4 records carry r4_* keys only; belt and braces""",
  """procs = [r for r in recs if 'row' in r and not r.get('r4') and not r.get('passcell')]  # R4 / passcell records carry r4_* / pc_* keys only; belt and braces"""),
 ("""bad = [r for r in procs if r.get('invalid')]""",
  """pcs = [r for r in recs if r.get('passcell')]
short = [r for r in pcs if r['short']]
print(f'pass-cells (ruling 8): {len(pcs)}, every slot clean {sum(1 for r in pcs if not r["short"])}, short {len(short)}, '
      f'gating (k_clean >= 3) {sum(1 for r in pcs if r["gates"])}, NOT gating {sum(1 for r in pcs if not r["gates"])}; re-runs spent '
      f'{sum(r["reruns"] for r in pcs)}, deepest re-run {max([r.get("rerun_no", 0) for r in procs] or [0])}')
for r in short:
    print(f'SHORT {r["block"]}-p{r["pass"]} {r["pc_row"]}#{r["pc_binary"]}@W{r["pc_W"]}: k_clean {r["k_clean"]} of {r["k_target"]} '
          f'(unclean rounds {r["unclean_rounds"]}, {r["reruns"]} re-runs){"" if r["gates"] else "  -> does not gate"}')
bad = [r for r in procs if r.get('invalid')]"""),
])
