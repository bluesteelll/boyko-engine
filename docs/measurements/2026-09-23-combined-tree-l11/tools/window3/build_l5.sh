#!/bin/bash
# Step B contingency: build the C4 (L5) runner from a detached worktree at $1 (sha8) into its own target dir.
# Usage: build_l5.sh <sha8>   -- creates D:/wt/mq-<sha8> and D:/wt/_targets/mq-<sha8>; never touches l5np/lighttable.
SHA="$1"; [ -n "$SHA" ] || { echo "usage: build_l5.sh <sha8>"; exit 2; }
export PATH="$HOME/.cargo/bin:$PATH"
unset RUSTFLAGS
export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=6
export CARGO_TARGET_DIR=D:/wt/_targets/mq-$SHA
if [ ! -d D:/wt/mq-$SHA ]; then
  git -C D:/wt/l5np worktree add --detach D:/wt/mq-$SHA $SHA || exit 1
fi
cd D:/wt/mq-$SHA || exit 1
echo "START $(date -Iseconds) cwd=$(pwd) HEAD=$(git rev-parse HEAD)"
echo "RUSTFLAGS=[${RUSTFLAGS:-unset}] CARGO_TARGET_DIR=$CARGO_TARGET_DIR RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN CARGO_INCREMENTAL=$CARGO_INCREMENTAL CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS"
rustc -vV | grep host
cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid 2>&1
rc=$?
echo "END $(date -Iseconds) exit=$rc"
ls -la $CARGO_TARGET_DIR/parity/deps/jolt_parity_pyramid-*.exe
exit $rc
