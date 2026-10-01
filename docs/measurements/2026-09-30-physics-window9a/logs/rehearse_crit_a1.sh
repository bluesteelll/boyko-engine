#!/usr/bin/env bash
# Audit-1 rehearsal of the criterion blocks with the FINAL driver: C4-BR (3 exes) + C4-G4, then C4-G4-kd on its own.
cd "$(dirname "$0")/.."
export W9A_TEST_HOT='none#none@W0:0:0'
W9A_TEST_RAW="$PWD/test/a1_crit" python -B tools/window9a_run.py --test --blocks C4-BR,C4-G4 > test/a1_crit.stdout 2>&1
echo "crit exit $?" > test/a1_crit.rc
W9A_TEST_RAW="$PWD/test/a1_kd" python -B tools/window9a_run.py --test --blocks C4-G4-kd > test/a1_kd.stdout 2>&1
echo "kd exit $?" > test/a1_kd.rc
