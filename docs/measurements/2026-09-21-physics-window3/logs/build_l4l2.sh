#!/bin/bash
export PATH="$HOME/.cargo/bin:$PATH"
unset RUSTFLAGS
export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=6
export CARGO_TARGET_DIR=D:/wt/_targets/mq-aac562a7
cd D:/wt/mq-aac562a7 || exit 1
echo "START $(date -Iseconds) cwd=$(pwd) HEAD=$(git rev-parse HEAD)"
echo "RUSTFLAGS=[${RUSTFLAGS:-unset}] CARGO_TARGET_DIR=$CARGO_TARGET_DIR RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN CARGO_INCREMENTAL=$CARGO_INCREMENTAL CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS"
cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid 2>&1
rc=$?
echo "END $(date -Iseconds) exit=$rc"
exit $rc
