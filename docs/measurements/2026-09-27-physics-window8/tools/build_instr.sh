#!/usr/bin/env bash
set -u
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
W="C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8"
cd D:/wt/_targets/w8s-trees/226bd99e || exit 9
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR=D:/wt/_targets/w8s-226bd99e
for B in jolt_parity_pyramid omega_b_region; do
  echo "=== $(date '+%F %T') bench $B"
  cargo bench --no-run --locked --profile parity -p boyko-physics --bench $B --message-format=json-render-diagnostics > "$W/logs/build_instr_$B.json" 
  echo "rc=$? $(date '+%F %T')"
done
