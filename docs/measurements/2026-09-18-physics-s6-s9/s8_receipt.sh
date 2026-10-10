#!/bin/bash
sha=$1
MQ=docs/measurements/2026-09-18-physics-s6-s9
cd D:/wt/mq-$sha || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_TARGET_DIR=D:/wt/_targets/mq-$sha
start=$(date +%s)
cargo test --release -p boyko-physics --bench sleeping_pipeline > $MQ/logs/$sha.s8receipt.out 2>&1
rc=$?
echo "rc=$rc secs=$(( $(date +%s) - start ))" >> $MQ/logs/$sha.s8receipt.out
