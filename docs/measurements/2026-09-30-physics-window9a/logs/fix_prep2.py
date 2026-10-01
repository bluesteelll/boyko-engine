p = 'PREP.md'
s = open(p, encoding='utf-8').read()
old = """**What the 65-min margin covers (audit-1 W1, `--dry-run` prints it: "RE-RUN PRICE"):** TOTAL leaves the re-run stages out. The MODELLED re-run time is **40 min** (window 8b's slot rate, 108 re-runs / 786 slots = 14 %, x the timed
wall; a model, unmeasured for the 5-8 min criterion processes) and fits in the margin. The WORST CASE, every pass spending its whole allowance, is **272 min** (AB 21, G5 7, BR 84, G4 132, kd 27) and does NOT fit: it is a bound,
not an expectation."""
new = """**What the 52-min margin covers (audit-1 W1, `--dry-run` prints it: "RE-RUN PRICE"; numbers as of 2026-09-30 with C4-RAPIER and BR at K 3 - the first cut said 65 min / 40 min / 272 min):** TOTAL leaves the re-run stages out. The MODELLED re-run time is **31 min** (window 8b's slot rate, 108 re-runs / 786 slots = 14 %, x the timed
wall; a model, unmeasured for the 5-8 min criterion processes) and fits in the margin. The WORST CASE, every pass spending its whole allowance, is **229 min** (AB 21, RAPIER 14, G5 7, BR 28, G4 132, kd 27) and does NOT fit: it is a bound,
not an expectation."""
assert s.count(old) == 1
s = s.replace(old, new)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
