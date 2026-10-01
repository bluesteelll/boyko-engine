import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
P = 'tools/window9a_run.py'
H9 = '''"""PHYSICS WINDOW 9a (2026-09-30) driver = window 8b's window8b_run.py (protocol verbatim) with window 9a's changes:
  * rows9a.json (+ the optional overlay rows9a.extra.json, merged by key as in 8b): the C4 blocks (C4-BR, C4-G4, C4-G4-kd,
    C4-AB with the in-block Jolt 5.6 row, C4-G5) and the reserved slot for the Rapier rows (kind 'rapier', PREP.md);
  * RULING 8 of RULINGS-2026-09-29-W8B (a K = 2 pass-cell does not gate; a dropped slot is re-run until its pass-cell has K =
    3 or the pass ends): the single end-of-pass re-run of 8b becomes a re-run STAGE of up to RERUN_MAX_WAVES waves inside a
    wall allowance of RERUN_STAGE_FRAC x the pass's own timed wall (the "pass ends" bound; the cutoff and the STOP flag
    still end everything). A wave re-runs every slot whose latest attempt is contaminated or invalid; a slot leaves the stage
    at its first clean valid attempt. Records: attempt 'rerun' (as in 8b) + rerun_no 1..; one {'passcell': ...} record per
    (pass, cell) with k_clean / k_target (no 'row' key, like the R4 records). The reduction's slot rule: the original if
    clean and valid, else the FIRST clean valid re-run, else the slot is dropped; a pass-cell with fewer than 3 clean
    slots does not gate;
  * binaries may carry expect_config {key: value} (checked against the runner SUMMARY's config on every process: an exe/key
    swap shows as tree_brute_max_rows 128 vs 144) besides sha256_pin;
  * kind 'rapier' (reserved): a runner-shaped external harness, data-driven validation (expect_summary dotted paths, "$W");
  * --test-rounds N (rehearsals of the re-run rule) and the test-only injection W9A_TEST_HOT.
Window 8b's own description follows.
'''
apply(P, [
 ('"""PHYSICS WINDOW 8b (2026-09-28) driver', H9 + 'PHYSICS WINDOW 8b (2026-09-28) driver'),
 ("AP.add_argument('--start', default='', help='dry-run only: the assumed start time HH:MM (default: now)')",
  "AP.add_argument('--start', default='', help='dry-run only: the assumed start time HH:MM (default: now)')\n"
  "AP.add_argument('--test-rounds', type=int, default=1, help='--test only: rounds per pass (default 1; 3 rehearses the re-run rule)')"),
 ("MAX_VOIDS_PER_PASS = 20\n",
  "MAX_VOIDS_PER_PASS = 20\n"
  "# Ruling 8 (RULINGS-2026-09-29-W8B): the re-run stage of a pass. \"The pass ends\" = one of these bounds, whichever comes first:\n"
  "# every pass-cell has all its slots clean; RERUN_MAX_WAVES waves have run; the stage has spent RERUN_STAGE_FRAC x the pass's\n"
  "# own timed wall (rounds x per-round estimate; window 8b's 108 re-runs were 14 % of its 786 slots, so 0.5 is generous).\n"
  "RERUN_MAX_WAVES = 4\n"
  "RERUN_STAGE_FRAC = 0.5\n"
  "MIN_K_GATE = 3             # a pass-cell with fewer clean slots than this does not gate (ruling 8, window 7's clause)\n"),
 ("ROWS_PATH = os.path.join(W8, 'rows9a.json')\nEXTRA_PATH = os.path.join(W8, 'rows9a.extra.json')\n",
  "ROWS_PATH = os.path.join(W8, 'rows9a.json')\n"
  "EXTRA_PATH = os.path.join(W8, 'rows9a.extra.json')\n"
  "if (TEST or DRY) and os.environ.get('W9A_EXTRA'):   # rehearsals only: a scratch overlay, never honoured in the window\n"
  "    EXTRA_PATH = os.environ['W9A_EXTRA']\n"
  "INJECT = {}                 # --test only: W9A_TEST_HOT='row#bin@W:round:n;...' forces the first n attempts of a slot hot\n"
  "if TEST and os.environ.get('W9A_TEST_HOT'):\n"
  "    for _spec in os.environ['W9A_TEST_HOT'].split(';'):\n"
  "        _cell, _rnd, _n = _spec.rsplit(':', 2)\n"
  "        _rid, _rest = _cell.split('#')\n"
  "        _key, _w = _rest.split('@W')\n"
  "        INJECT[(_rid, _key, int(_w), int(_rnd))] = int(_n)\n"),
])
