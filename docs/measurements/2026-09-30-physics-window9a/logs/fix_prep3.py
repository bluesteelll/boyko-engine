p = 'PREP.md'
s = open(p, encoding='utf-8').read()
old = "(`python -B tools/rehearsal_summary9a.py <dir>`). (Pass-cells of a 1-round rehearsal have K 1 and \"do not gate\": an artefact, as before.)"
new = ("(`python -B tools/rehearsal_summary9a.py <dir>`). Through the shell launcher: `W9A_TEST_HOT=none#none@W0:0:0 bash run_window9a.sh --test --blocks C4-RAPIER`: WINDOW_DONE `exit 0 / complete`, counts exit 0, 20 timed + 2 warm-ups, 0 invalid "
       "(kept as test/rapier9a/shell_rehearsal). (Pass-cells of a 1-round rehearsal have K 1 and \"do not gate\": an artefact, as before.)")
assert s.count(old) == 1
s = s.replace(old, new)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
