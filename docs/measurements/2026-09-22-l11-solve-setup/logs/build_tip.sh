#!/bin/bash
cd /d/wt/lighttable || exit 9
echo "START $(date -Iseconds) pwd=$(pwd) HEAD=$(git rev-parse HEAD)"
git status --short | head -5
PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=6 TMP=D:/wt/_targets/tmp TEMP=D:/wt/_targets/tmp CARGO_TARGET_DIR=D:/wt/_targets/vkval-msvc cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid 2>&1
echo "EXIT $? $(date -Iseconds)"
