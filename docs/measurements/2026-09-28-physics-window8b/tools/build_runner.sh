#!/usr/bin/env bash
# Window 8b: build one exported tree's parity runner (and optionally omega) in its own target dir.
#   build_runner.sh <tree dir> <target dir> <log> [extra --bench names...]
set -u
T="$1"; TGT="$2"; LOG="$3"; shift 3
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 \
  TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR="$TGT"
cd "$T" || exit 9
sha256sum Cargo.lock > "$LOG.lock_before"
{ echo "# $(date '+%F %T') tree $T target $TGT"; rustc -Vv; } > "$LOG"
cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid "$@" >> "$LOG" 2>&1
RC=$?
sha256sum Cargo.lock > "$LOG.lock_after"
echo "# $(date '+%F %T') exit $RC" >> "$LOG"
exit $RC
