#!/usr/bin/env bash
# Window 7 build of ONE commit's parity runner, in an exported tree (never in a worktree).
# usage: build_one.sh <sha> [bench-name]
set -u
S="$1"
BENCH="${2:-jolt_parity_pyramid}"
W=C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7
T="$W/trees/$S"
LOG="$W/logs/build_${S}_${BENCH}.log"
: > "$LOG"
echo "[$(date '+%F %T')] export $S" >> "$LOG"
if [ ! -f "$T/Cargo.toml" ]; then
  mkdir -p "$T"
  git -C D:/wt/lighttable archive "$S" | tar -x -C "$T" || { echo "EXPORT FAILED" >> "$LOG"; exit 2; }
fi
cp D:/wt/joltab/Cargo.lock "$T/Cargo.lock"
echo "lock copied (LF sha256): $(tr -d '\r' < "$T/Cargo.lock" | sha256sum | cut -c1-64)" >> "$LOG"
echo "[$(date '+%F %T')] build start" >> "$LOG"
cd "$T" && PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 \
  TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR="D:/wt/_targets/win7-$S" \
  cargo bench --no-run --profile parity -p boyko-physics --bench "$BENCH" >> "$LOG" 2>&1
RC=$?
echo "[$(date '+%F %T')] build exit $RC" >> "$LOG"
echo "lock after build (LF sha256): $(tr -d '\r' < "$T/Cargo.lock" | sha256sum | cut -c1-64)" >> "$LOG"
exit $RC
