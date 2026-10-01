#!/usr/bin/env bash
# Builds the three arms of the harness and copies them to gate/bin/ (the builds share one target
# directory, so each overwrites target/release/rapier-parity.exe; the copies keep all three).
#   rapier-parity-simd8.exe  default features: rapier3d parallel + simd8          (8 lanes, timed)
#   rapier-parity-simd4.exe  --no-default-features --features simd4: parallel     (4 lanes, timed)
#   rapier-parity-det.exe    --no-default-features --features det: parallel +
#                            enhanced-determinism                                 (4 lanes, NOT timed)
# Every cargo call passes --locked: the lockfile is the provenance, it must not be re-resolved.
# Stable msvc toolchain, release profile (lto = "fat", codegen-units = 1), x86-64-v3.
# Also writes gate/bin/SHA256SUMS (the exes), gate/bin/SOURCES.sha256 (what they were built from)
# and gate/bin/features_<arm>.txt (`cargo tree -e features --locked`: the resolved feature set).
set -eu
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
export CARGO_TARGET_DIR=D:/wt/_targets/rapier-parity
export RUSTFLAGS="-C target-cpu=x86-64-v3"
JOBS="${JOBS:-8}"
mkdir -p gate/bin

build_arm() { # build_arm <arm> <cargo feature flags...>
    local arm="$1"; shift
    cargo build --release --locked -j "$JOBS" "$@"
    cp "$CARGO_TARGET_DIR/release/rapier-parity.exe" "gate/bin/rapier-parity-$arm.exe"
    cargo tree --locked -e features -i rapier3d "$@" > "gate/bin/features_$arm.txt"
    cargo tree --locked -e features -i parry3d "$@" >> "gate/bin/features_$arm.txt"
}

build_arm simd8
build_arm simd4 --no-default-features --features simd4
build_arm det --no-default-features --features det

sha256sum gate/bin/rapier-parity-simd8.exe gate/bin/rapier-parity-simd4.exe gate/bin/rapier-parity-det.exe \
    > gate/bin/SHA256SUMS
sha256sum src/main.rs Cargo.toml Cargo.lock build.sh .cargo/config.toml > gate/bin/SOURCES.sha256
cat gate/bin/SHA256SUMS gate/bin/SOURCES.sha256
