#!/usr/bin/env bash
# Window 9a, 2026-10-01 (block C4-CGU): the TIP parity runner rebuilt with a different codegen-units, everything else the
# recipe of tools/build_runner.sh (stable msvc 1.98.1, --locked, RUSTFLAGS unset, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=8,
# --profile parity, the tree's own .cargo/config.toml). The ONLY difference is the environment override
# CARGO_PROFILE_PARITY_CODEGEN_UNITS (cargo's spelling of [profile.parity] codegen-units; the tree is never edited).
#   build_tipcgu1.sh <tree dir> <target dir> <log> [<codegen-units> | default]
#   default = the override is NOT set (the tree's own parity profile = release = 16 units): the control build.
# `-v` is appended so the log holds the rustc command lines (proof that -C codegen-units reached the crates); verbosity is not
# part of any fingerprint. The wall time of the whole build (cold target dir) is appended to the log.
set -u
T="$1"; TGT="$2"; LOG="$3"; CGU="${4:-1}"
unset CARGO_PROFILE_PARITY_CODEGEN_UNITS
if [ "$CGU" != "default" ]; then export CARGO_PROFILE_PARITY_CODEGEN_UNITS="$CGU"; fi
S=$(date +%s)
bash "$(dirname "${BASH_SOURCE[0]}")/build_runner.sh" "$T" "$TGT" "$LOG" -v
RC=$?
E=$(date +%s)
echo "# build wall $((E - S)) s rc $RC codegen-units ${CGU} (CARGO_PROFILE_PARITY_CODEGEN_UNITS=${CARGO_PROFILE_PARITY_CODEGEN_UNITS:-<unset>})" >> "$LOG"
exit $RC
