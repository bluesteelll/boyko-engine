#!/bin/bash
# usage: verbose_arm.sh <sha8> <tag>  -- the literal ISA-parity command, verbose, all bench targets
sha=$1; tag=$2
MQ=docs/measurements/2026-09-18-physics-s6-s9
cd D:/wt/mq-$sha || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_TARGET_DIR=D:/wt/_targets/mq-$sha
start=$(date +%s)
echo "env RUSTFLAGS=[${RUSTFLAGS-unset}] CARGO_ENCODED_RUSTFLAGS=[${CARGO_ENCODED_RUSTFLAGS-unset}] toolchain=$RUSTUP_TOOLCHAIN target=$CARGO_TARGET_DIR" > $MQ/logs/$sha.$tag.stderr
cargo bench -p boyko-physics --no-run -v >> $MQ/logs/$sha.$tag.stderr 2>&1
rc=$?
echo "rc=$rc secs=$(( $(date +%s) - start ))" >> $MQ/logs/$sha.$tag.stderr
exit $rc
