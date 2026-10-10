#!/usr/bin/env bash
# Window 7 wave 2, Q3: build the G4 refinement INSTRUMENT (never committed) from an exported trunk tree.
# The tree is `git -C D:/wt/lighttable archive 93b2615b | tar -x` with ONE line changed: benches/broadphase.rs
# `const G4_SIZES` (the recipe's own remedy, g4_g5_recipe.md:131 "add the sizes between them to G4_SIZES (a one-line
# edit)"). The diff is variant/g4ref.diff. Bench profile, as window 6's bpbench_983480a9.exe.
set -u
W=C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7b
S=93b2615bcea4873e2d3af6b560c6e7997740f67f
T="$W/trees/${S}_g4ref"
LOG="$W/logs/build_g4ref_broadphase.log"
: > "$LOG"
AVAIL=$(df -BG --output=avail /d | tail -1 | tr -dc '0-9')
echo "[$(date '+%F %T')] D: free ${AVAIL} GB" >> "$LOG"
if [ "$AVAIL" -lt 15 ]; then echo "STOP: D: under 15 GB" >> "$LOG"; exit 3; fi
echo "lock (LF sha256): $(tr -d '\r' < "$T/Cargo.lock" | sha256sum | cut -c1-64)" >> "$LOG"
echo "variant line: $(grep -n '^const G4_SIZES' "$T/crates/boyko_physics/benches/broadphase.rs")" >> "$LOG"
echo "[$(date '+%F %T')] build start" >> "$LOG"
cd "$T" && PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 \
  TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR="D:/wt/_targets/win7b-g4ref" \
  cargo bench --no-run -p boyko-physics --bench broadphase >> "$LOG" 2>&1
RC=$?
echo "[$(date '+%F %T')] build exit $RC" >> "$LOG"
echo "lock after build (LF sha256): $(tr -d '\r' < "$T/Cargo.lock" | sha256sum | cut -c1-64)" >> "$LOG"
exit $RC
