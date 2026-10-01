#!/usr/bin/env bash
# Window 8b: the G4 criterion exe (tree-f3/window_cmds.md §1), from the g4ref export (G4_SIZES edited, bin/g4ref.patch).
set -u
LOG="$1"
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 \
  TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR=D:/wt/_targets/w8b-g4ref
cd D:/wt/_targets/w8b-trees/g4ref || exit 9
sha256sum Cargo.lock > "$LOG.lock_before"
{ echo "# $(date '+%F %T') tree g4ref target w8b-g4ref"; rustc -Vv; } > "$LOG"
cargo bench --no-run --locked -p boyko-physics --bench broadphase >> "$LOG" 2>&1
RC=$?
sha256sum Cargo.lock > "$LOG.lock_after"
echo "# $(date '+%F %T') exit $RC" >> "$LOG"
exit $RC
