#!/bin/bash
# usage: build_arm.sh <sha8> <tag> <bench>...
# Builds the named bench targets of boyko-physics in D:/wt/mq-<sha8> into D:/wt/_targets/mq-<sha8>.
sha=$1; tag=$2; shift 2
MQ=docs/measurements/2026-09-18-physics-s6-s9
args=()
for b in "$@"; do args+=(--bench "$b"); done
cd D:/wt/mq-$sha || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_TARGET_DIR=D:/wt/_targets/mq-$sha
start=$(date +%s)
cargo bench -p boyko-physics "${args[@]}" --no-run --message-format=json-render-diagnostics \
  > $MQ/logs/$sha.$tag.json 2> $MQ/logs/$sha.$tag.stderr
rc=$?
echo "rc=$rc secs=$(( $(date +%s) - start ))" >> $MQ/logs/$sha.$tag.stderr
exit $rc
