#!/usr/bin/env bash
# V2: the parity runner under the overlap-only rule - the runner named by $RUNNER_D0_EXE with
# `--speculative-distance 0 --speculative-velocity-cap 0` appended to every invocation that has
# arguments (a bare invocation is the runner's self check, which takes none). The exit code is
# the runner's; 127 when RUNNER_D0_EXE is unset or not a file (outside the runner's own codes
# 0/2/3/4/101, so a row that wants 2 cannot pass on a mis-armed wrapper).
if [ -z "${RUNNER_D0_EXE:-}" ] || [ ! -f "$RUNNER_D0_EXE" ]; then echo "runner_d0.sh: RUNNER_D0_EXE is unset or not a file ('${RUNNER_D0_EXE:-}')" >&2; exit 127; fi
if [ $# -eq 0 ]; then exec "$RUNNER_D0_EXE"; fi
exec "$RUNNER_D0_EXE" "$@" --speculative-distance 0 --speculative-velocity-cap 0
