#!/usr/bin/env bash
# Window 9a: the G4 criterion exe from the g4rT export (TIP + bin/g4ref.patch), bench profile (window 8b's build_g4ref.sh,
# paths as arguments):  build_g4rT.sh <tree dir> <target dir> <log>
set -u
T="$1"; TGT="$2"; LOG="$3"
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 \
  TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR="$TGT"
cd "$T" || exit 9
sha256sum Cargo.lock > "$LOG.lock_before"
{ echo "# $(date '+%F %T') tree $T target $TGT"; rustc -Vv; } > "$LOG"
cargo bench --no-run --locked -p boyko-physics --bench broadphase >> "$LOG" 2>&1
RC=$?
sha256sum Cargo.lock > "$LOG.lock_after"
echo "# $(date '+%F %T') exit $RC" >> "$LOG"
exit $RC
