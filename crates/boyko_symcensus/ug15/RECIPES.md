# UG-15 instrument recipes, and the UG-08 / UG-09 recipes re-established at B3

Measured on the msvc gate host on 2026-09-23 (rung B3, stage 1). Receipts: `receipts/b3/`.

## The prefix, and two traps

Every command runs from the worktree root with the build prefix exported in the same shell:

```bash
cd <worktree> && export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc \
  CARGO_TARGET_DIR=<target dir> CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=5 TMP=<tmp> TEMP=<tmp>
cargo build -q -p boyko-symcensus --bin ug15
UG15="$CARGO_TARGET_DIR/debug/ug15.exe"
```

- **Export, then invoke.** A one-line `VAR=… cargo build && ug15.exe …` gives the environment to cargo
  only; `ug15` then REDs with `CARGO_TARGET_DIR is unset` (critique O1).
- **Git Bash rewrites a bare `/Word` argument into a Windows path.** `-- -C link-arg=/Brepro` reached
  rustc as `link-arg=C:/Program Files/Git/Brepro` (measured). Pass `/…` linker flags from bash only with
  `MSYS_NO_PATHCONV=1`. `ug15`'s own `/MAP:` and `/Brepro` arguments are built inside the process.
- `ug15` is invoked directly, never through `cargo run`, so no outer cargo holds the build lock while
  the nested `cargo rustc` runs in the same target dir.
- `ug15` refuses to run from a tree other than the one it was compiled from, with an unset or relative
  `CARGO_TARGET_DIR`, or against a target dir whose `.ug15-host` marker names another host.

## UG-15 commands (`src/bin/ug15.rs`)

| command | what it does |
|---|---|
| `$UG15 tools` | resolves and versions every tool; RED on any absence (overrides: `BOYKO_UG15_LLVM_BIN` = a directory, `BOYKO_UG15_SYMBOLIZER` = a file; each is the only place searched when set) |
| `$UG15 build --subject S [--profile P] [--map F] [-- <rustc args>]` | one forced, paired build; prints the object, image, digests, link line and final rustc line |
| `$UG15 build … --control-drop-emit-obj` / `--control-no-touch` | the instrument's red controls O (no post-LTO object from this build) and P (a fresh unit) |
| `$UG15 nm <file>` | control E: on a msvc image this is `RED [empty census] … no symbols` |
| `$UG15 rlibs --subject S` | leg (7b)'s member formats of every census-crate rlib |
| `$UG15 leg7 snapshot --subject S [--profile seam-census] --out F` | leg (7) (a) sections, (b) object multiset, (c) exports |
| `$UG15 leg7 compare PARENT CHILD [--rename F]` | strict leg (7); RED on any difference |
| `$UG15 leg7 compare PARENT CHILD --expect NAME [--expect NAME]` | a red CONTROL (critique C1): passes only if every NAME is among the child-only (b) entries |
| `$UG15 textcheck --subject S --seam SNAPSHOT` | `.text` of `release` equals the seam-census snapshot's |
| `$UG15 probe i\|iii\|iv\|v …` | B3's probe build (`receipts/b3/probe-*.txt`, `receipts/b3/PROBES.md`) |

Subjects: `boyko_demo`, `clear` (boyko-app's example), `swap_remove`, `query_dsl`, `phase9_scheduler`.

**Pairing.** Every build bumps the modification time of the target's root source (content unchanged),
requires cargo's JSON `fresh: false` for the final unit, and requires the object and the image both
written after the bump; every reader re-checks both stamps after reading. Nothing in the target dir is
ever deleted by the instrument.

**Profiles.** `seam-census` is `inherits = "release"` alone (the plan's `strip = "none"` changed codegen
through cargo's `-C metadata`, measured, see the root `Cargo.toml`); `sensitivity-map` adds
`debug = "line-tables-only"`, `strip = "none"`; `bench-shipped` is `inherits = "release"` alone.

## UG-08 — Miri on `nightly-x86_64-pc-windows-msvc` (adopted form: the joined key)

```bash
cd <worktree> && env -u RUSTUP_TOOLCHAIN -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
  PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=<miri target dir> CARGO_INCREMENTAL=0 \
  MIRIFLAGS="<the target header's MIRIFLAGS, copied in full> -Zmiri-strict-provenance" \
  timeout 1200 cargo +nightly-x86_64-pc-windows-msvc miri test \
  --config 'target."cfg(windows)".rustflags=["-Zrandomize-layout"]' -p <crate> --test <file>
```

- **SB leg:** the same with `-Zmiri-tree-borrows` removed from `MIRIFLAGS`. Never
  `-Zmiri-retag-fields` or `-Zmiri-unique-is-unique`.
- **Why the joined key** (`receipts/b3/ug08.txt`): `RUSTFLAGS=-Zrandomize-layout` replaces
  `.cargo/config.toml`'s `-C target-cpu=x86-64-v3` (the ISA census REDs under Miri: 5 of 5 features
  absent); the joined key keeps it and adds the flag (both on the `-v` rustc line; the census passes).
  With the baseline present, Miri 2026-09-09 runs the AVX2 arms (`sdf_simd::o9_kernel_tests::x8_*`
  pass).
- **Known blocker, outside B3:** under `-Zrandomize-layout` (either form) `boyko-physics`'s lib does not
  build under Miri: `WarmSeedStats` (`src/row_identity.rs:346`) is `repr(Rust)` with a
  `size_of == 32` const assert, which randomized layout legitimately breaks.
- Pass: `running N test(s)` with N the file's count and `0 filtered out`; receipts name the nightly
  date and hash; one named target per invocation, capped at 20 minutes.

## UG-09 — loom (`receipts/b3/ug09.txt`)

```bash
cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' test --release \
  -p boyko-threadpool --test loom_pool -- --list          # exactly the six model names
LOOM_MAX_PREEMPTIONS=3 cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' test --release \
  -p boyko-threadpool --test loom_pool -- --test-threads=1 --exact \
  loom_m1_fork_join_no_lost_wakeup loom_m2_idle_race_c_no_lost_wakeup \
  loom_m2_calibration_no_producer_fence_is_lost loom_m2b_idle_cas_contention_exactly_one \
  loom_m3_shutdown_handshake_worker_exits loom_m1c_count_gated_completion_wakes_the_parked_worker_joiner
# must print `running 6 tests` and `6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
cargo --config 'build.rustflags=["--cfg","loom"]' test --release -p boyko-threadpool --test loom_pool -- --list
# negative arm: `0 tests` — build.rustflags is ignored whenever a [target.*] entry matches
```
