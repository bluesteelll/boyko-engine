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
| `$UG15 leg1 census` | leg (1) by hand: what the walk read and what it counted (the gate is the test below) |
| `$UG15 leg1 deps [--write]` | the census crates' direct dependency set; `--write` re-pins `pins/leg1-deps.pins` after review |
| `$UG15 leg2 capture` | **B3 only**: builds the five subjects under `release`, writes `pins/leg2-<subject>.pins`, `pins/leg2-absent.txt` and `pins/leg2-frozen.tsv` (the frozen set, one row per pinned body) |
| `$UG15 leg2 check [--rename F] [--named F] [--subjects a,b]` | strict leg (2): first, before any build, RED `pin set` unless every pin file holds exactly its rows of `pins/leg2-frozen.tsv` and every (candidate, subject) pair is frozen or in `pins/leg2-absent.txt` (a deleted file or block is never a smaller green; `--subjects` narrows the builds, not this); then, per built subject (every subject the candidate list names, with or without a pin file), RED `pin set` on a disposition its object contradicts: a pair recorded absent whose rules now locate a symbol ("recorded absent, but present"), or a name an `Exact`/`Contains` rule of a pinned candidate locates that is not frozen ("located, but not frozen"; `FirstWithPrefix` is exempt, cut D5); then RED `symbol absent` on a pinned name with no symbol, RED `moved` on an unnamed body change (a move whose bodies differ only in content-named constants, `__xmm@…`, stays RED and carries a `hint:` line: TypeId literals follow the resolved dependency graph, so compare `Cargo.lock` first); `--named` lists `<cand or name>\t<reason>` for an attributed rung. The receipt's `Cargo.lock` line is the sha256, over LF text (a CRLF checkout of the tracked lock hashes the same), of the workspace lock the objects were built under (a second line if cargo re-resolved it during the builds) |
| `$UG15 leg7b --subject boyko_demo` | leg (7b) over the census rlibs' object members (bitcode), plus leg (1) over the census build scripts' generated `OUT_DIR` sources |
| `$UG15 map a capture` | map (a) under `sensitivity-map` (route a1): `maps/file-map.tsv`, and the carrier column of `pins/leg2-absent.txt` |
| `$UG15 map b capture --arm L --out F` / `map b diff --type T A B` | map (b) on a layout control branch; `diff` rewrites T's rows of `maps/layout-map.tsv` |
| `$UG15 map c capture\|check` | map (c): `maps/containment.tsv` from the leg-(1) walker; `check` REDs on any drift (re-capture in the rung's merge) |

**The light legs are tests** (UG-01 runs them on every rung):

```bash
cargo test -p boyko-symcensus --test ug15_leg1_source_census --test ug15_leg1_shapes \
  --test ug15_leg3_size_of --test ug15_leg5_seam_inventory --test ug15_legs_4_6_na \
  --test normalize_fixtures --test dev_only_edge -- --nocapture
cargo test --release -p boyko-symcensus --test ug15_leg3_size_of -- --nocapture   # leg (3) in release
cargo test -p boyko-macros --lib ug15_corpus -- --nocapture                        # M-b
```

Subjects: `boyko_demo`, `clear` (boyko-app's example), `swap_remove`, `query_dsl`, `phase9_scheduler`.

**Pairing.** Every build bumps the modification time of the target's root source (content unchanged),
requires cargo's JSON `fresh: false` for the final unit, and requires the object and the image both
written after the bump; every reader re-checks both stamps after reading. Nothing in the target dir is
ever deleted by the instrument.

**Profiles.** `seam-census` is `inherits = "release"` alone (the plan's `strip = "none"` changed codegen
through cargo's `-C metadata`, measured, see the root `Cargo.toml`); `sensitivity-map` adds
`debug = "line-tables-only"`, `strip = "none"`; `bench-shipped` is `inherits = "release"` alone.

## IB V11 commands (IB P0r)

V11 is IB's game-unit identity. `ug15` builds the parity runner (`boyko-physics-parity`, bin
`jolt_parity_pyramid`) and the game-shaped probe (`boyko-physics-probe`, bin `physics_probe`) under
`release`, retained per build. Windows (msvc) only.

**Rebuild `ug15` before every IB command**: `cargo build -p boyko-symcensus --bin ug15 --locked`. An
executable built from other sources than the tree holds is `RED [stale instrument]` before any build.
This is because the instrument key K covers the bytes compiled into it (`src/key.rs`).

| command | what it does |
|---|---|
| `$UG15 unit --retain DIR [--controls RECEIPT] [--out F]` | V11 (a), (b), (c), (e), (f) on this tree, runner against probe, then the certification. Exit 0 only when (a), (b), (c), (f) are green and the instrument is certified. A red (e) narrows the scope note and is never the verdict (D14) |
| `$UG15 unit-compare PRE TIP` | V11 (d1)-(d4) between two `unit` receipts, under the closed normalisation list (`src/unit.rs` module docs) |
| `$UG15 unit-controls --retain DIR --scratch DIR [--receipt-out F] [--only ids]` | the red controls RC-11 (a)-(f), RC-13 and RC-14 (a)-(c), and the green controls G-b and G-d. Writes the control receipt. Clean tree only |
| `$UG15 unit-arm --arm reference\|runner\|bench --retain DIR --out F` | a scratch tree's builds for one control, recorded and not evaluated (`unit-controls` spawns it in each scratch tree) |

**Retention.** `--retain` (and `--scratch`) must be absolute, outside the tree and the target dir, and
empty or absent. Each build is copied to `<retain>/<tag>/` (image, object, pdb, and the `/MAP` the
linker writes there), hashed, and checked intact before the next build starts. Tags come from a
closed vocabulary: `runner`, `runner-<pin>`, `probe`, `probe-<pin>`, `probe-wide`, `plain-<tag>`,
`rc<NN><x>-<tag>`.

**One separator form (orchestrator ruling 61).** `--scratch`, `CARGO_TARGET_DIR` and the tree root
must each use one separator, `/` or `\`. (d2) substitutes the target dir and the root in their two
single-separator forms, so a mixed path would read as a difference on every line. A mixed
`--scratch` is RED before any build; every IB run is RED on a mixed target dir or root; and
`unit-compare` refuses a receipt that carries one. The scratch trees' paths are appended in
`--scratch`'s own form (a `Path::join` on Windows mixed them at C7's first run).

**Codegen units (orchestrator ruling 57).** With codegen-units unset, `--emit=obj` makes rustc
codegen one unit. Every IB build therefore passes `-C codegen-units=<n>`, where n is the profile's
effective value: its own `codegen-units`, or the nearest one up `inherits`, else rustc's
non-incremental default of 16. After the build, the final unit's rustc line must carry that token
exactly once, or twice when the profile sets it and cargo passes it as well. Every value must equal n,
and the line must carry no `-C incremental`; anything else is RED. On IB's two subjects under
`release` this reproduced the plain `cargo build --release` `.text` byte for byte at IB P0r's RF-5a,
and V11 (c2) re-checks it by bytes on every `unit` run. It is a per-subject proof, not a property of
the flag: finding F2 below measured two subjects where the explicit value is not the shipped bytes.
So the `ib` lane re-runs V11 (c2) on its own subjects at its base and STOPs if it is red (orchestrator
ruling 77 (2), PC-PT1-11).

**The UG-15 legs' scope: codegen-units = 1 objects (orchestrator rulings 71 (4), 74 (A), 77 (1)).**
Every leg above builds through the same `--emit=obj` with codegen-units unset, so each one censuses a
codegen-units = 1 object, not the shipped codegen. Their pins and receipts are identity claims between
two trees built identically at that codegen, never claims about the shipped image's bytes. This covers
leg (2)'s frozen pins, leg (7)'s snapshots, `ug15 textcheck` (both of its sides are such builds), and
B3's probe (iv) and PC-18 `.text` numbers (`receipts/b3/`).

PHYS-TOOL-1 sampled the shipped-codegen question without editing this crate (K unchanged). A scratch
driver over this library built each leg (2) subject once under `release` through IB's
`Request::codegen_units` = 16, the profile's effective value, then ran leg (2)'s own `pins::check_with`
on that object. It found two things:

- **F1: the pins are codegen-specific bytes.** At the explicit 16, the claim holds on `clear`,
  `boyko_demo` and `phase9_scheduler`, and differs on `swap_remove` (H-2's criterion harness bodies
  move) and `query_dsl` (H-1's `EcsMaster::run_system::<…>` is inlined into `Bencher::iter`). All
  five hold under the legs' own recipe on the same tree, so this is not drift since the capture.
- **F2: an explicit `-C codegen-units=16` is not the shipped codegen.** On `boyko_demo` and `clear`
  it does not reproduce the plain `cargo build --release` `.text` byte for byte. The explicit flag
  itself causes this, not `--emit=obj`: on `boyko_demo`, `cargo rustc --release -- -C
  codegen-units=16` without `--emit=obj` gives the leg's `.text` too.

Their files: `docs/physics/perf-campaign/levers/tooling/01-PHYS-TOOL-1-RECORD.md` section 4 (the
receipts' hashes, committed). The raw receipts are under `D:/tmp/phys-orch/phys-tool-1/`:
`u4/leg2-<subject>-ship.txt`, `u4/base/`, `u4/repro/`, `u4/builds.txt`, `u4/img/`, `u4/d2/`, and the
driver `driver/`.

**Deferred (PC-PT1-10).** Answering the shipped-codegen question needs an instrument that builds the
EXACT shipped recipe, with no explicit codegen-units flag, and compares two arms built that way. It joins the unified
plan's F2 end-state gates (`docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`, Phase F). Until
then no leg is re-pinned, and none claims the shipped bytes. `src/objbuild.rs`'s module docs still
call this an owed decision, because editing them would change K (PC-PT1-12).

**Certification (W13).** `unit` is certified by a control receipt `ug15-ib-controls v1` that names
K. The receipt must be complete (not `partial`), and every required control must have its expected
verdict. Two locations satisfy the rule:

- the committed `ug15/receipts/ib/controls-<K8>.txt`, tracked and clean in this tree;
- `--controls <path>` outside both trees: the window directory, the prep fallback.

`--only` writes a partial receipt, which never certifies. K is sha256 over this crate's `Cargo.toml`
and `src/**` (`\r\n` normalised), `rustc -vV` of the compiler cargo invokes, and `cargo -V`. Any edit
under `src/` changes K, so `unit-controls` must be re-run.

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
