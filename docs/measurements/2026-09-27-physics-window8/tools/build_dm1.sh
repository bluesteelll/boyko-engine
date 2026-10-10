#!/usr/bin/env bash
# usage: build_dm1.sh <A|B> ; builds dm1_material_table_timing in D:/wt/_dm1_ab at its current HEAD
set -u
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
SIDE=$1
W="C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8"
cd D:/wt/_dm1_ab || exit 9
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR=D:/wt/_targets/dm1-ab
echo "=== $(date '+%F %T') side $SIDE HEAD $(git rev-parse HEAD)"
cargo test -p boyko-app --test dm1_material_table_timing --no-run --locked --message-format=json > "$W/logs/build_dm1_$SIDE.json" 2> "$W/logs/build_dm1_$SIDE.stderr"
echo "rc=$? $(date '+%F %T')"
