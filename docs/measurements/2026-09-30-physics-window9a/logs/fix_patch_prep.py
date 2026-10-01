p = 'logs/patch_prep.py'
s = open(p, encoding='utf-8').read()
pairs = [
    ("D. 52 extra mutations x 4 twins", "D. 51 extra mutations x 4 twins (204 cases)"),
    ("F. six driver-only cases (launch args carry --receipt, an empty fixture, a hang flag, steps 12 vs 500 rows, JSON true at W1, `test=True` skips V7 only)",
     "F. seven driver-only cases (launch args carry --receipt, an empty fixture, a hang flag, steps 12 vs 500 rows, JSON true at W1, `test=True` skips V7 only, `test=True` still flags V1)"),
    ("the validator on the 20 REAL gate cells (all green) and 40+ mutations", "the validator on the 20 REAL gate cells (all green) and 40 mutations"),
    ("**Cross-check against the reference (tools/xcheck_rapier9a.py -> gate/xcheck_rapier9a.txt): 1,279 comparisons, 0 disagreeing**",
     "**Cross-check against the reference (tools/xcheck_rapier9a.py -> gate/xcheck_rapier9a.txt): 1,279 checks, 0 disagreeing**"),
    ("so ours `J-T` and a Rapier cell are up to ~50 min apart in wall time.",
     "so ours `J-T` (C4-AB, 20:00-20:50 in the dry-run timeline) and a Rapier cell (C4-RAPIER, 20:50-21:25) are from a few minutes to ~70 min apart in wall time."),
    ("(written from the rules' text; the previous data-driven `expect_summary` schema is gone)",
     "(written from the rules' text; the driver no longer reads the previous data-driven `expect_summary` / `expect_present` fields, which rows9a.extra.example.json, a placement template, still shows)"),
]
for a, b in pairs:
    assert s.count(a) == 1, a[:60]
    s = s.replace(a, b)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
