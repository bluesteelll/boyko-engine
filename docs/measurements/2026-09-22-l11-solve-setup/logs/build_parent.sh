#!/bin/bash
SP="/c/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4b"
cd "$SP/parent_tree" || exit 9
echo "START $(date -Iseconds) pwd=$(pwd)"
PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=6 TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR=D:/wt/_targets/l11-parent-msvc cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid 2>&1
echo "EXIT $? $(date -Iseconds)"
