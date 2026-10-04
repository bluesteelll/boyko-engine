#!/usr/bin/env bash
# V2: the parity runner under the overlap-only rule - the runner named by $RUNNER_D0_EXE with
# `--speculative-distance 0 --speculative-velocity-cap 0` appended to every invocation that has
# arguments (a bare invocation is the runner's self check, which takes none). The exit code is
# the runner's.
if [ $# -eq 0 ]; then exec "$RUNNER_D0_EXE"; fi
exec "$RUNNER_D0_EXE" "$@" --speculative-distance 0 --speculative-velocity-cap 0
