#!/usr/bin/env python3
"""The merge-check CI mirror: every job of `.github/workflows/ci.yml` that this Windows host can
run, run here, and a PASS / FAIL / NOT-RUNNABLE table at the end.

Why it exists (orchestrator ruling 96, 2026-10-22). GitHub CI run 39663719484 on `integ/unified`
97900f68 failed 13 of 23 jobs on two defects no merge check could see: items only the Linux target
reports as dead (`crates/boyko_app/src/post_host.rs`) and two allows `scripts/check_hotpath_exceptions.py`
had no rows for. Both jobs run on `ubuntu-latest`; every lane check ran on Windows only.

WHAT RUNS, by the row's kind in the JOB TABLE below (one row per ci.yml job, or per matrix leg):

  script              the job's `python3 <script>` line, with `python3` replaced by this interpreter.
  linux               the job's cargo line(s) as a CROSS CHECK: `--target x86_64-unknown-linux-gnu
                      --keep-going` added, the CI's RUSTFLAGS from the yml (the one place this script
                      sets RUSTFLAGS), in `--linux-target-dir` (a matrix job's own `CARGO_TARGET_DIR`
                      becomes a sub-directory of it). `feature-legs` runs the yml's own derivation
                      step (its Python, verbatim but for one asserted `--manifest-path`) and checks
                      every derived leg.
  linux-compile-only  the compile half of a job whose other half (a link, a run) needs Linux.
  host                the job's `run:` blocks verbatim in Git Bash, in `--host-target-dir`.
  host-heavy          as `host`, but only with `--heavy` (hours, a workspace test run).
  not-runnable        printed as NOT-RUNNABLE with the row's reason.

Commands always come from ci.yml at run time and are never re-listed here. Every adapter asserts
the shape it reads; a job whose shape changed exits 2 ("adapter cannot read job X"), so a reshaped
job reds the mirror and is never skipped silently. A ci.yml job with no row, a stale row, or a
matrix leg no row covers exits 2 as well, and `tests/ci_mirror_census.rs` fails the ordinary test
run on the same drift.

RUSTFLAGS. Only the Linux rows get it (ruling 96: with `--target` it reaches only that triple's
artifacts, in their own directory). Host rows never see RUSTFLAGS: CI's `-D warnings` is emulated
by failing a row whose output carries cargo's `warning: `<pkg>` (<target>) generated N warning(s)`
summary (`(manifest)` excepted, it does not fail CI either). Two host rows keep a CI flag through
the per-target config key the loom recipe uses, never `build.rustflags`: `loom`'s first step gets the
job's RUSTFLAGS as `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS` in its own directory (the second
step already unsets RUSTFLAGS and passes `--config`). `miri` runs on the host target with the
config's `target-cpu` (CI's job RUSTFLAGS replaces it, and a per-target key cannot: Cargo joins
those with the config's), so Miri interprets `boyko_ecs`'s AVX2 arms here and the scalar arms on
CI; the row names the files every run, so the deviation stays visible.

STATUSES: PASS, FAIL, NOT-RUNNABLE (the row's reason), SKIPPED (not selected: names the flag),
COVERED (another row covers it: names the row and the computed reason). Exit 0 = no FAIL, 1 = some
FAIL, 2 = mirror drift or a failed preflight. "PARTIAL MIRROR" is printed whenever a row was SKIPPED.

MACHINE GUARDS: rows run one at a time; before each, free commit >= --min-free-commit-gb (waits in
2-minute slices up to 60 min, then FAILs the row) and free disk >= --min-free-disk-gb (else exit 2);
the disk floor is re-read every 30 s while a row runs, and a row that crosses it is killed and the
run stops (exit 2); a row past its wall budget has ONLY its own process tree killed
(`taskkill /T /F /PID`).

Usage (the caller's environment carries RUSTUP_TOOLCHAIN, PATH, CARGO_BUILD_JOBS, TMP; the caller
takes any machine-wide lock):

    python tools/ci_mirror.py --linux-target-dir D:/wt/_targets/linux-check --log-dir <dir>
        [--host-target-dir <dir>] [--heavy] [--only a,b] [--skip a,b] [--list] [--self-test]
        [--fap-target-dir <dir>] [--min-free-commit-gb 6] [--min-free-disk-gb 15]
        [--row-timeout-min 120] [--heavy-row-timeout-min 480]

Files in --log-dir: PID, progress.tsv (one line per finished row), summary.tsv, DONE (the exit
code), one log per row, and dead_items.tsv (every never-used / unused / unfulfilled-expectation
diagnostic of the Linux rows: leg, file:line, message). A --log-dir may be re-used: a row's log is
truncated at the row's first write in a run, so a verdict never reads an earlier run's output (a row
this run does not reach keeps its old log, and no line of summary.tsv points at it).
"""

from __future__ import annotations

import argparse
import ctypes
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_CI_YML = REPO / ".github" / "workflows" / "ci.yml"
LINUX_TRIPLE = "x86_64-unknown-linux-gnu"
HOST_TRIPLE = "x86_64-pc-windows-msvc"

# Feature legs the cross check cannot build: `libmimalloc-sys`'s build script needs an
# `x86_64-linux-gnu-gcc`, which the ubuntu runner has and this host does not (WORKFLOW.md, Feature
# axis). Each must be a DERIVED leg, else exit 2; each is reported NOT-RUNNABLE and checked natively
# (msvc, same flags, no `--target`) as a substitute.
CROSS_UNBUILDABLE = {
    ("boyko-ecs", "bench-alloc"),
    ("boyko-physics-parity", "bench-alloc"),
}

# The kinds a row may carry, closed. `tests/ci_mirror_census.rs` holds the same list.
KINDS = ("script", "linux", "linux-compile-only", "host", "host-heavy", "not-runnable")
KINDS_NEEDING_A_NOTE = ("linux-compile-only", "host-heavy", "not-runnable")
NOTE_MIN = 40

# The actions a job may use. Anything else is a step this script cannot reproduce: exit 2.
KNOWN_ACTIONS = re.compile(r"^(actions/checkout@v4|dtolnay/rust-toolchain@[0-9A-Za-z.\-]+|Swatinem/rust-cache@v2)$")

# Environment a host or Linux row must never inherit from the caller; the yml's own env is applied
# afterwards.
SCRUBBED_ENV = (
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "MIRIFLAGS",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "CARGO_TARGET_DIR",
)
SCRUBBED_PREFIXES = ("BOYKO_", "CARGO_TARGET_X86_64_")

# CI's `-D warnings` on rows that do not get RUSTFLAGS: cargo prints this summary for every unit
# with rustc warnings, replayed ones included. `(manifest)` is a Cargo.toml note, not a rustc
# warning, and it does not fail CI (run 39663719484's `check` log printed one and went on).
WARNING_SUMMARY = re.compile(r"^warning: `[^`]+` \((?!manifest\))[^)]*\) generated \d+ warnings?")

# What F4's class list is made of: rustc's dead / unused diagnostics.
DIAG_HEAD = re.compile(r"^(error|warning)(?:\[[A-Za-z0-9_]+\])?: (.+)$")
DIAG_AT = re.compile(r"^\s*--> (.+?):(\d+):(\d+)\s*$")
DEAD_MESSAGE = re.compile(
    r"never (used|read|constructed)|^unused |unreachable (code|pattern|expression)"
    r"|value assigned to .* is never read|lint expectation is unfulfilled"
)

# --- JOB TABLE ---------------------------------------------------------------------------------
# One row per ci.yml job (`id`) or per matrix leg (`id:leg`): `id | kind | note`. A bare row covers
# every leg of a matrix job. The note is required (>= 40 characters) for the kinds that do not run
# the whole job. ADDING A JOB TO ci.yml = ONE ROW HERE; `tests/ci_mirror_census.rs` says so.
# === CI-MIRROR JOB TABLE BEGIN ===
JOB_TABLE = r"""
hot-path-exceptions | script |
doc-contracts | script |
check | linux |
test | host-heavy | Workspace-wide `cargo test`, debug and release: each leg is hours on this host and a full target dir; run with --heavy (the final merge to master, or a lane that touches timing tests).
test-linux:debug | not-runnable | Running Linux test binaries needs a Linux runtime and this host has none (no WSL). Its compile half is the `check` row's selection at the dev profile, which the mirror verifies and that row checks.
test-linux:release | linux-compile-only | The run half needs a Linux runtime (no WSL here). The compile half is the release line with `test` -> `check`: the only Linux check of the `cfg(not(debug_assertions))` set.
profile-legs | linux |
profile-census | host |
reflect-on | host |
reflect-census | host |
reflect-dogfood | host-heavy | Builds the render stack a second time with `reflect` on (boyko_render, boyko_rhi_vulkan): an hour-class build and its own artifacts; run with --heavy.
clippy | linux |
feature-legs | linux |
feature-count-tests | host |
bench-compile | linux-compile-only | `cargo bench --no-run` links, and this host has no Linux linker or glibc. The compile half is `check --profile bench --lib --bins --benches`, the same unit set.
force-alloc-panic | host-heavy | A second release workspace test run. COVERED by `test:release` while no `.rs` reads `cfg(force_alloc_panic)` and the job's RUSTFLAGS differ from the top level by that flag alone; else run with --fap-target-dir.
miri | host | Host target (CI interprets the Linux one), and the config's `target-cpu` reaches the build where CI's job RUSTFLAGS drops it, so Miri interprets boyko_ecs's AVX2 arms here; the row names them.
loom | host | Both steps natively in <host target dir>/ci-mirror-loom; step 1's job RUSTFLAGS ride the per-target key CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS (the loom recipe), never `build.rustflags`.
"""
# === CI-MIRROR JOB TABLE END ===


class Drift(Exception):
    """The mirror cannot read ci.yml, or its table no longer matches it: exit 2."""


# ------------------------------------------------------------------------------------------------
# A reader for the YAML subset ci.yml is written in: block mappings and sequences, `|` / `|-` block
# scalars, one-line flow lists, plain and quoted scalars, full-line comments. Anything else raises,
# so a construct this reader does not know is a Drift, never a silently wrong job.
# ------------------------------------------------------------------------------------------------

KEY_RE = re.compile(r"^([A-Za-z0-9_][A-Za-z0-9_.\-]*):(?:[ \t]+(.*))?$")


def _strip_plain_comment(text: str) -> str:
    m = re.search(r"\s#", text)
    return (text[: m.start()] if m else text).rstrip()


def _scalar(raw: str, lineno: int):
    raw = raw.strip()
    if not raw:
        return None
    if raw[0] == '"':
        out, i = [], 1
        while i < len(raw):
            c = raw[i]
            if c == "\\" and i + 1 < len(raw):
                nxt = raw[i + 1]
                out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(nxt, "\\" + nxt))
                i += 2
                continue
            if c == '"':
                rest = raw[i + 1 :].strip()
                if rest and not rest.startswith("#"):
                    raise Drift(f"ci.yml line {lineno}: text after a quoted scalar")
                return "".join(out)
            out.append(c)
            i += 1
        raise Drift(f"ci.yml line {lineno}: unterminated double-quoted scalar")
    if raw[0] == "'":
        out, i = [], 1
        while i < len(raw):
            if raw[i] == "'":
                if raw[i + 1 : i + 2] == "'":
                    out.append("'")
                    i += 2
                    continue
                rest = raw[i + 1 :].strip()
                if rest and not rest.startswith("#"):
                    raise Drift(f"ci.yml line {lineno}: text after a quoted scalar")
                return "".join(out)
            out.append(raw[i])
            i += 1
        raise Drift(f"ci.yml line {lineno}: unterminated single-quoted scalar")
    if raw[0] == "[":
        body = _strip_plain_comment(raw)
        if not body.endswith("]"):
            raise Drift(f"ci.yml line {lineno}: a flow list must close on its own line -- extend the reader")
        inner = body[1:-1].strip()
        if any(ch in inner for ch in "[]{}"):
            raise Drift(f"ci.yml line {lineno}: a nested flow collection -- extend the reader")
        return [_scalar(item, lineno) for item in inner.split(",")] if inner else []
    if raw[0] in "{&*!>|%@`":
        raise Drift(f"ci.yml line {lineno}: unsupported YAML construct {raw[0]!r} -- extend the reader")
    return _strip_plain_comment(raw)


class _Reader:
    def __init__(self, text: str):
        self.lines = text.replace("\r\n", "\n").split("\n")

    def indent(self, i: int) -> int:
        line = self.lines[i]
        return len(line) - len(line.lstrip(" "))

    def next_sig(self, i: int):
        while i < len(self.lines):
            stripped = self.lines[i].strip()
            if stripped and not stripped.startswith("#"):
                if "\t" in self.lines[i][: self.indent(i) + 1]:
                    raise Drift(f"ci.yml line {i + 1}: a tab in the indentation")
                return i
            i += 1
        return None

    def block(self, i: int, ind: int):
        s = self.lines[i].strip()
        if s == "-" or s.startswith("- "):
            return self.sequence(i, ind)
        return self.mapping(i, ind)

    def _value_after_key(self, j: int, ind: int, rest: str):
        if rest == "" or rest.startswith("#"):
            k = self.next_sig(j + 1)
            if k is not None and self.indent(k) > ind:
                return self.block(k, self.indent(k))
            return None, j + 1
        if rest[0] in "|>":
            header = _strip_plain_comment(rest)
            if header not in ("|", "|-"):
                raise Drift(f"ci.yml line {j + 1}: block scalar header {header!r} -- extend the reader")
            return self.block_scalar(j + 1, ind, header)
        return _scalar(rest, j + 1), j + 1

    def mapping(self, i: int, ind: int):
        out: dict = {}
        while True:
            j = self.next_sig(i)
            if j is None:
                return out, len(self.lines)
            if self.indent(j) < ind:
                return out, j
            if self.indent(j) > ind:
                raise Drift(f"ci.yml line {j + 1}: unexpected indentation")
            s = self.lines[j].strip()
            m = KEY_RE.match(s)
            if not m:
                raise Drift(f"ci.yml line {j + 1}: expected `key:` -- extend the reader")
            key, rest = m.group(1), (m.group(2) or "").strip()
            if key in out:
                raise Drift(f"ci.yml line {j + 1}: duplicate key {key!r}")
            out[key], i = self._value_after_key(j, ind, rest)

    def sequence(self, i: int, ind: int):
        out: list = []
        while True:
            j = self.next_sig(i)
            if j is None:
                return out, len(self.lines)
            if self.indent(j) < ind:
                return out, j
            if self.indent(j) > ind:
                raise Drift(f"ci.yml line {j + 1}: unexpected indentation")
            s = self.lines[j].strip()
            if not (s == "-" or s.startswith("- ")):
                raise Drift(f"ci.yml line {j + 1}: a key beside a sequence -- extend the reader")
            rest = s[1:].strip()
            if rest == "":
                k = self.next_sig(j + 1)
                if k is None or self.indent(k) <= ind:
                    out.append(None)
                    i = j + 1
                    continue
                val, i = self.block(k, self.indent(k))
            elif KEY_RE.match(rest):
                # `- key: value` opens a mapping whose keys sit two columns in.
                self.lines[j] = " " * (ind + 2) + rest
                val, i = self.mapping(j, ind + 2)
            else:
                val, i = _scalar(rest, j + 1), j + 1
            out.append(val)

    def block_scalar(self, i: int, parent_ind: int, header: str):
        body: list[str] = []
        base = None
        while i < len(self.lines):
            line = self.lines[i]
            if line.strip() == "":
                body.append("")
                i += 1
                continue
            ind = self.indent(i)
            if ind <= parent_ind:
                break
            if base is None:
                base = ind
            if ind < base:
                raise Drift(f"ci.yml line {i + 1}: a block scalar line left of its first line")
            body.append(line[base:])
            i += 1
        while body and body[-1] == "":
            body.pop()
        text = "\n".join(body)
        if header == "|" and body:
            text += "\n"
        return text, i


def parse_yaml(text: str):
    r = _Reader(text)
    i = r.next_sig(0)
    if i is None:
        return {}
    if r.indent(i) != 0:
        raise Drift("ci.yml: the document does not start at column 0")
    value, i = r.block(i, 0)
    j = r.next_sig(i)
    if j is not None:
        raise Drift(f"ci.yml line {j + 1}: content after the document")
    return value


SELF_TEST_YAML = """\
name: CI
on:
  push:
    branches: [master]
  workflow_dispatch:

env:
  # a comment inside a mapping
  RUSTFLAGS: "-D warnings -C target-cpu=x86-64-v3"
jobs:
  # a comment between jobs
  alpha:
    name: alpha (${{ matrix.profile }})
    runs-on: windows-latest
    defaults:
      run:
        shell: bash
    strategy:
      fail-fast: false
      matrix:
        profile: [debug, release]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.98.1
        with:
          components: rust-src, llvm-tools
      - name: a block scalar
        run: |
          if [ "${{ matrix.profile }}" = "release" ]; then
            echo 'release' # a shell comment stays
          fi

          echo done
      # a comment between steps
      - name: step env
        shell: bash
        env:
          MIRIFLAGS: '-Zmiri-strict-provenance'
        run: cargo test --locked -p x -- --nocapture
  beta:
    runs-on: ubuntu-latest
    steps:
      - run: python3 scripts/x.py
"""

SELF_TEST_EXPECTED = {
    "name": "CI",
    "on": {"push": {"branches": ["master"]}, "workflow_dispatch": None},
    "env": {"RUSTFLAGS": "-D warnings -C target-cpu=x86-64-v3"},
    "jobs": {
        "alpha": {
            "name": "alpha (${{ matrix.profile }})",
            "runs-on": "windows-latest",
            "defaults": {"run": {"shell": "bash"}},
            "strategy": {"fail-fast": "false", "matrix": {"profile": ["debug", "release"]}},
            "steps": [
                {"uses": "actions/checkout@v4"},
                {"uses": "dtolnay/rust-toolchain@1.98.1", "with": {"components": "rust-src, llvm-tools"}},
                {
                    "name": "a block scalar",
                    "run": 'if [ "${{ matrix.profile }}" = "release" ]; then\n'
                    "  echo 'release' # a shell comment stays\nfi\n\necho done\n",
                },
                {
                    "name": "step env",
                    "shell": "bash",
                    "env": {"MIRIFLAGS": "-Zmiri-strict-provenance"},
                    "run": "cargo test --locked -p x -- --nocapture",
                },
            ],
        },
        "beta": {"runs-on": "ubuntu-latest", "steps": [{"run": "python3 scripts/x.py"}]},
    },
}

# The force-alloc-panic COVERED rule counts readers of `cfg(force_alloc_panic)`: code whose meaning
# the flag changes. (source, the `file:line` readers it must report, what the case is.)
FAP_SCAN_CASES = (
    ("#[cfg(force_alloc_panic)]\nfn f() {}", ["x.rs:1"], "a cfg attribute"),
    ("#[cfg(all(\n    test,\n    force_alloc_panic,\n))]\nfn f() {}", ["x.rs:3"], "a cfg predicate split over lines"),
    ("fn f() -> bool { cfg!(force_alloc_panic) }", ["x.rs:1"], "a cfg! macro"),
    ("#[cfg_attr(not(force_alloc_panic), ignore)]\nfn f() {}", ["x.rs:1"], "a cfg_attr predicate"),
    ("#[cfg_attr(test, cfg_attr(\n    force_alloc_panic, ignore))]\nfn f() {}", ["x.rs:2"], "a nested cfg_attr"),
    (
        "fn f<'a>(s: &'a str) -> char { '\"' }\n#[cfg(force_alloc_panic)]\nfn g() {}",
        ["x.rs:2"],
        "a lifetime, and a char literal that holds a quote",
    ),
    ('let s = "a \\" quote"; #[cfg(force_alloc_panic)] fn g() {}', ["x.rs:1"], "an escaped quote in a string"),
    (
        '#[ignore = "workstation: read 0.46 vs 0.41 ns/iter in its force_alloc_panic leg"]\nfn f() {}',
        [],
        "an ignore reason (a string literal)",
    ),
    ('const S: &str = "#[cfg(force_alloc_panic)]";', [], "a cfg predicate inside a string literal"),
    ('const S: &str = r#"cfg!(force_alloc_panic) "quoted""#;', [], "a raw string"),
    ('#[cfg_attr(test, doc = "force_alloc_panic")]\nfn f() {}', [], "a cfg_attr's attribute string"),
    (
        "// #[cfg(force_alloc_panic)]\n/// cfg!(force_alloc_panic)\n/* cfg(force_alloc_panic) /* nested */ cfg(force_alloc_panic) */\nfn f() {}",
        [],
        "line, doc and nested block comments",
    ),
    ("--cfg force_alloc_panic in prose", [], "a `--cfg` mention"),
    ("let force_alloc_panic = 1;", [], "an identifier outside a cfg predicate"),
)


def self_test() -> list[str]:
    failures: list[str] = []
    try:
        got = parse_yaml(SELF_TEST_YAML)
        if got != SELF_TEST_EXPECTED:
            failures.append(f"reader fixture: got {got!r}")
    except Drift as e:
        failures.append(f"reader fixture raised: {e}")
    for bad, what in (
        ("a:\n  b: {x: 1}\n", "a flow mapping"),
        ("a:\n  b: >\n    folded\n", "a folded scalar"),
        ("a:\n  b: [x,\n    y]\n", "a multi-line flow list"),
        ("a: 1\na: 2\n", "a duplicate key"),
    ):
        try:
            parse_yaml(bad)
            failures.append(f"reader accepted {what}")
        except Drift:
            pass
    rows, errors = parse_table("x | linux |\ny:a | not-runnable | " + "n" * NOTE_MIN + "\n")
    if errors or [r.id for r in rows] != ["x", "y:a"]:
        failures.append(f"table fixture: {rows!r} {errors!r}")
    _, errors = parse_table("x | nonsense |\n")
    if not errors:
        failures.append("table accepted an unknown kind")
    _, errors = parse_table("y | not-runnable | short\n")
    if not errors:
        failures.append("table accepted a short note")
    if not WARNING_SUMMARY.match("warning: `boyko-app` (lib test) generated 2 warnings"):
        failures.append("warning summary regex misses a lib-test summary")
    if WARNING_SUMMARY.match("warning: `reflect-fixture` (manifest) generated 1 warning"):
        failures.append("warning summary regex counts a manifest note")
    if not DEAD_MESSAGE.search("function `report_post_plugin_absent` is never used"):
        failures.append("dead-item regex misses `is never used`")
    try:
        docs, code = split_derive_step(
            "cat > feature-refusals.txt <<'EOF'\na/b|why\nEOF\n"
            "cat > feature-scopes.txt <<'EOF'\nEOF\n"
            "python3 - <<'PY'\nprint(1)\nPY\n"
        )
        if docs != {"feature-refusals.txt": "a/b|why\n", "feature-scopes.txt": ""} or code != "print(1)":
            failures.append(f"derive-step fixture: {docs!r} {code!r}")
    except Drift as e:
        failures.append(f"derive-step fixture raised (an empty heredoc must parse): {e}")
    try:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "row.log"
            for text in (b"run 1: error: static `X` is never used\n", b"run 2: clean\n"):
                logs = RowLogs()
                for chunk in (text, b"step 2\n"):
                    with logs.open(log) as f:
                        f.write(chunk)
            if log.read_bytes() != b"run 2: clean\nstep 2\n":
                failures.append(f"row logs: a second run read back {log.read_bytes()!r} (an earlier run's output leaks into its verdict)")
    except OSError as e:
        failures.append(f"row logs fixture raised: {e}")
    for src, want, what in FAP_SCAN_CASES:
        got = fap_readers_in(src, "x.rs")
        if got != want:
            failures.append(f"force_alloc_panic reader scan, {what}: got {got}, want {want}")
    return failures


# ------------------------------------------------------------------------------------------------
# The job table and the job model
# ------------------------------------------------------------------------------------------------


@dataclass
class Row:
    id: str
    kind: str
    note: str


def parse_table(text: str) -> tuple[list[Row], list[str]]:
    rows: list[Row] = []
    errors: list[str] = []
    seen: set[str] = set()
    for line in text.splitlines():
        if "|" not in line:
            continue
        parts = [p.strip() for p in line.split("|", 2)]
        if len(parts) != 3:
            errors.append(f"table row {line!r}: need `id | kind | note`")
            continue
        rid, kind, note = parts
        if not re.fullmatch(r"[a-z][a-z0-9-]*(:[A-Za-z0-9_.\-]+)?", rid):
            errors.append(f"table row {rid!r}: malformed id")
        if kind not in KINDS:
            errors.append(f"table row {rid!r}: kind {kind!r} is not one of {', '.join(KINDS)}")
        if kind in KINDS_NEEDING_A_NOTE and len(note) < NOTE_MIN:
            errors.append(f"table row {rid!r}: a `{kind}` row needs a note of >= {NOTE_MIN} characters")
        if rid in seen:
            errors.append(f"table row {rid!r}: duplicate")
        seen.add(rid)
        rows.append(Row(rid, kind, note))
    return rows, errors


@dataclass
class Step:
    name: str | None
    uses: str | None
    run: str | None
    shell: str | None
    env: dict
    with_: dict


@dataclass
class Job:
    id: str
    name: str
    runs_on: str
    env: dict
    default_shell: str | None
    matrix_key: str | None
    matrix_values: list[str]
    steps: list[Step]
    continue_on_error: bool


@dataclass
class Unit:
    """One output row: a job, or one leg of a matrix job."""

    id: str
    job: Job
    leg: str | None
    row: Row


JOB_KEYS = {"name", "runs-on", "continue-on-error", "defaults", "strategy", "env", "steps"}
STEP_KEYS = {"name", "uses", "with", "run", "shell", "env"}
EXPR = re.compile(r"\$\{\{\s*(.*?)\s*\}\}")


def _as_env(value, where: str) -> dict:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise Drift(f"{where}: `env` is not a mapping")
    return {k: "" if v is None else str(v) for k, v in value.items()}


def load_jobs(ci_yml: Path) -> tuple[dict, dict[str, Job]]:
    doc = parse_yaml(ci_yml.read_text(encoding="utf-8"))
    if not isinstance(doc, dict) or not isinstance(doc.get("jobs"), dict):
        raise Drift("ci.yml has no `jobs:` mapping")
    top_env = _as_env(doc.get("env"), "top level")
    jobs: dict[str, Job] = {}
    for jid, raw in doc["jobs"].items():
        if not isinstance(raw, dict):
            raise Drift(f"job {jid}: not a mapping")
        unknown = set(raw) - JOB_KEYS
        if unknown:
            raise Drift(f"job {jid}: keys {sorted(unknown)} -- extend the reader")
        shell = None
        defaults = raw.get("defaults")
        if defaults is not None:
            if set(defaults) != {"run"} or set(defaults["run"] or {}) - {"shell"}:
                raise Drift(f"job {jid}: `defaults` beyond `run.shell` -- extend the reader")
            shell = (defaults["run"] or {}).get("shell")
        mkey, mvals = None, []
        strategy = raw.get("strategy")
        if strategy is not None:
            if set(strategy) - {"fail-fast", "matrix"}:
                raise Drift(f"job {jid}: strategy keys {sorted(strategy)} -- extend the reader")
            matrix = strategy.get("matrix")
            if matrix is not None:
                if not isinstance(matrix, dict) or len(matrix) != 1:
                    raise Drift(f"job {jid}: a matrix with other than one key -- extend the reader")
                (mkey, mvals), = matrix.items()
                if not isinstance(mvals, list) or not mvals or not all(isinstance(v, str) for v in mvals):
                    raise Drift(f"job {jid}: matrix `{mkey}` is not a flow list of values -- extend the reader")
        steps = []
        for n, s in enumerate(raw.get("steps") or []):
            if not isinstance(s, dict):
                raise Drift(f"job {jid} step {n}: not a mapping")
            unknown = set(s) - STEP_KEYS
            if unknown:
                raise Drift(f"job {jid} step {n}: keys {sorted(unknown)} (an `if:` is among those this mirror refuses) -- extend the reader")
            uses = s.get("uses")
            if uses is not None and not KNOWN_ACTIONS.match(uses):
                raise Drift(f"job {jid} step {n}: action {uses!r} is not one this mirror can reproduce")
            if (uses is None) == (s.get("run") is None):
                raise Drift(f"job {jid} step {n}: needs exactly one of `uses` / `run`")
            steps.append(
                Step(s.get("name"), uses, s.get("run"), s.get("shell"), _as_env(s.get("env"), f"job {jid} step {n}"), s.get("with") or {})
            )
        job = Job(
            jid,
            raw.get("name") or jid,
            raw.get("runs-on") or "",
            _as_env(raw.get("env"), f"job {jid}"),
            shell,
            mkey,
            list(mvals),
            steps,
            str(raw.get("continue-on-error")).lower() == "true",
        )
        _check_expressions(job)
        jobs[jid] = job
    return top_env, jobs


def _check_expressions(job: Job) -> None:
    texts = [job.name, *job.env.values()]
    for s in job.steps:
        texts += [s.name or "", s.run or "", *s.env.values()]
    for text in texts:
        for expr in EXPR.findall(text):
            if job.matrix_key is None or expr != f"matrix.{job.matrix_key}":
                raise Drift(f"job {job.id}: expression `${{{{ {expr} }}}}` is not one this mirror evaluates")


def substitute(text: str, job: Job, leg: str | None) -> str:
    if job.matrix_key is None:
        return text
    return text.replace("${{ matrix.%s }}" % job.matrix_key, leg or "")


def build_units(jobs: dict[str, Job], rows: list[Row]) -> list[Unit]:
    by_id = {r.id: r for r in rows}
    errors: list[str] = []
    units: list[Unit] = []
    claimed: set[str] = set()
    for job in jobs.values():
        legs = job.matrix_values if job.matrix_key else [None]
        for leg in legs:
            uid = job.id if leg is None else f"{job.id}:{leg}"
            leg_row = by_id.get(uid) if leg is not None else None
            bare = by_id.get(job.id)
            if leg_row and bare:
                errors.append(f"{uid}: both a bare row `{job.id}` and a leg row -- keep one")
                continue
            row = leg_row or bare
            if row is None:
                errors.append(f"{uid}: ci.yml job{' leg' if leg else ''} with no row in tools/ci_mirror.py's JOB TABLE")
                continue
            claimed.add(row.id)
            units.append(Unit(uid, job, leg, row))
    for r in rows:
        if r.id not in claimed:
            errors.append(f"{r.id}: a JOB TABLE row with no ci.yml job or leg -- delete the stale row")
        if r.kind in ("linux", "linux-compile-only"):
            jid = r.id.split(":")[0]
            if jid in jobs and jobs[jid].runs_on != "ubuntu-latest":
                errors.append(f"{r.id}: a `{r.kind}` row on a `{jobs[jid].runs_on}` job")
    if errors:
        raise Drift("the JOB TABLE does not match ci.yml:\n  " + "\n  ".join(errors))
    return units


# ------------------------------------------------------------------------------------------------
# Running
# ------------------------------------------------------------------------------------------------


class _MemStatus(ctypes.Structure):
    _fields_ = [
        ("dwLength", ctypes.c_ulong),
        ("dwMemoryLoad", ctypes.c_ulong),
        ("ullTotalPhys", ctypes.c_ulonglong),
        ("ullAvailPhys", ctypes.c_ulonglong),
        ("ullTotalPageFile", ctypes.c_ulonglong),
        ("ullAvailPageFile", ctypes.c_ulonglong),
        ("ullTotalVirtual", ctypes.c_ulonglong),
        ("ullAvailVirtual", ctypes.c_ulonglong),
        ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
    ]


def free_commit_gb() -> float | None:
    if os.name != "nt":
        return None
    status = _MemStatus()
    status.dwLength = ctypes.sizeof(_MemStatus)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        return None
    return status.ullAvailPageFile / 2**30


class RowLogs:
    """The row logs one run writes. A row's verdict, warning summaries and dead items are read back
    from its whole log, so the first open of a log in a run truncates it: a re-used --log-dir never
    feeds an earlier run's output into this run's table. Later opens in the same run (a row's next
    step) append."""

    def __init__(self) -> None:
        self.started: set[Path] = set()

    def open(self, log: Path):
        log.parent.mkdir(parents=True, exist_ok=True)
        key = log.resolve()
        mode = "ab" if key in self.started else "wb"
        self.started.add(key)
        return open(log, mode)


@dataclass
class Result:
    status: str
    exit: int | None = None
    seconds: float = 0.0
    log: str = ""
    detail: str = ""
    sub: list[tuple[str, str, str]] = field(default_factory=list)


class Mirror:
    def __init__(self, args, ci_yml: Path):
        self.args = args
        self.ci_yml = ci_yml
        self.top_env, self.jobs = load_jobs(ci_yml)
        rows, errors = parse_table(JOB_TABLE)
        if errors:
            raise Drift("the JOB TABLE is malformed:\n  " + "\n  ".join(errors))
        self.units = build_units(self.jobs, rows)
        self.log_dir: Path | None = Path(args.log_dir).resolve() if args.log_dir else None
        self.linux_dir = Path(args.linux_target_dir).resolve() if args.linux_target_dir else None
        host = args.host_target_dir or os.environ.get("CARGO_TARGET_DIR")
        self.host_dir = Path(host).resolve() if host else None
        self.cargo = None
        self.bash = None
        self.row_logs = RowLogs()
        self.results: dict[str, Result] = {}
        self.dead: list[tuple[str, str, str]] = []
        # Set when a row was killed for disk space: the run stops after that row (exit 2).
        self.disk_stop: str | None = None

    # --- environment -----------------------------------------------------------------------------

    def base_env(self) -> dict:
        env = {
            k: v
            for k, v in os.environ.items()
            if k.upper() not in SCRUBBED_ENV and not k.upper().startswith(SCRUBBED_PREFIXES)
        }
        # A toolchain this host does not have must never be downloaded behind the caller's back.
        env["RUSTUP_AUTO_INSTALL"] = "0"
        env["CARGO_TERM_COLOR"] = "never"
        return env

    def yml_env(self, job: Job, leg: str | None, step: Step | None, keep_rustflags: bool) -> dict:
        env: dict = {}
        for layer in (self.top_env, job.env, step.env if step else {}):
            for k, v in layer.items():
                env[k] = substitute(v, job, leg)
        if not keep_rustflags:
            env.pop("RUSTFLAGS", None)
        # Output stays parseable; colour is presentation only.
        env["CARGO_TERM_COLOR"] = "never"
        return env

    # --- process ----------------------------------------------------------------------------------

    def run(self, argv: list[str], env: dict, log: Path, timeout_min: float) -> tuple[int, float]:
        start = time.monotonic()
        with self.row_logs.open(log) as out:
            out.write(f"\n$ {' '.join(argv)}\n".encode())
            out.flush()
            proc = subprocess.Popen(argv, cwd=REPO, env=env, stdout=out, stderr=subprocess.STDOUT)
            deadline = start + timeout_min * 60 if timeout_min > 0 else None
            next_disk_check = start
            while True:
                try:
                    rc = proc.wait(timeout=2)
                    break
                except subprocess.TimeoutExpired:
                    now = time.monotonic()
                    why = None
                    if deadline is not None and now > deadline:
                        why = f"row past its {timeout_min} min budget"
                    elif self.args.min_free_disk_gb > 0 and now >= next_disk_check:
                        # A cold row can write tens of GB; the guard before the row cannot see that.
                        next_disk_check = now + 30
                        free = self.disk_free_gb()
                        if free < self.args.min_free_disk_gb:
                            why = f"disk guard: {free:.1f} GB free, below --min-free-disk-gb {self.args.min_free_disk_gb}"
                            self.disk_stop = why
                    if why:
                        # Only this row's own process tree; never a by-name kill.
                        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
                        proc.wait()
                        out.write(f"\n[ci-mirror] {why}: process tree {proc.pid} killed\n".encode())
                        return 124, time.monotonic() - start
        return rc, time.monotonic() - start

    @staticmethod
    def read(log: Path) -> str:
        return log.read_text(encoding="utf-8", errors="replace") if log.exists() else ""

    def disk_free_gb(self) -> float:
        """Free space on the drive the build output goes to (the Linux dir's, else the host dir's)."""
        target = next((p for p in (self.linux_dir, self.host_dir) if p is not None), REPO)
        return shutil.disk_usage(Path(target.anchor)).free / 2**30

    def guard(self) -> str | None:
        """Machine guards before a row. Returns a FAIL reason, or raises Drift to stop the run."""
        need_disk = self.args.min_free_disk_gb
        if need_disk > 0:
            for d in {p for p in (self.linux_dir, self.host_dir) if p is not None}:
                anchor = d if d.exists() else Path(d.anchor)
                free = shutil.disk_usage(anchor).free / 2**30
                if free < need_disk:
                    raise Drift(f"disk guard: {free:.1f} GB free on {d.anchor}, below --min-free-disk-gb {need_disk}")
        need_mem = self.args.min_free_commit_gb
        if need_mem > 0:
            waited = 0
            while True:
                free = free_commit_gb()
                if free is None or free >= need_mem:
                    return None
                if waited >= 60:
                    return f"free commit {free:.1f} GB stayed below {need_mem} GB for 60 min"
                print(f"[ci-mirror] free commit {free:.1f} GB < {need_mem} GB: waiting 2 min", flush=True)
                time.sleep(120)
                waited += 2

    # --- the dead-item list (F4's class list) ------------------------------------------------------

    def collect_dead(self, leg: str, text: str) -> int:
        lines = text.splitlines()
        found = 0
        for i, line in enumerate(lines):
            m = DIAG_HEAD.match(line)
            if not m or not DEAD_MESSAGE.search(m.group(2)):
                continue
            where = "?"
            for nxt in lines[i + 1 : i + 4]:
                at = DIAG_AT.match(nxt)
                if at:
                    where = f"{at.group(1).replace(chr(92), '/')}:{at.group(2)}"
                    break
            self.dead.append((leg, where, m.group(2)))
            found += 1
        return found

    @staticmethod
    def warning_summaries(text: str) -> list[str]:
        return [l for l in text.splitlines() if WARNING_SUMMARY.match(l)]

    # --- adapters ---------------------------------------------------------------------------------

    def run_steps(self, job: Job) -> list[Step]:
        return [s for s in job.steps if s.run is not None]

    def single_cargo_line(self, job: Job, leg: str | None) -> list[str]:
        steps = self.run_steps(job)
        if len(steps) != 1:
            raise Drift(f"adapter cannot read job {job.id}: expected one `run:` step, found {len(steps)}")
        text = substitute(steps[0].run, job, leg).strip()
        if "\n" in text or not text.startswith("cargo "):
            raise Drift(f"adapter cannot read job {job.id}: its `run:` is not a single cargo line")
        return shlex.split(text)

    def release_line(self, job: Job, leg: str) -> list[str]:
        """The cargo line a `test`-shaped job runs for `leg`: one `if`/`else` over the matrix value."""
        steps = self.run_steps(job)
        if len(steps) != 1:
            raise Drift(f"adapter cannot read job {job.id}: expected one `run:` step")
        lines = [l.strip() for l in substitute(steps[0].run, job, leg).splitlines() if l.strip()]
        shape = re.fullmatch(r'if \[ "(\w+)" = "(\w+)" \]; then', lines[0]) if lines else None
        if not shape or len(lines) != 5 or lines[2] != "else" or lines[4] != "fi":
            raise Drift(f"adapter cannot read job {job.id}: not the two-branch `if [ profile = release ]` shape")
        pick = lines[1] if shape.group(1) == shape.group(2) else lines[3]
        if not pick.startswith("cargo "):
            raise Drift(f"adapter cannot read job {job.id}: a branch is not a cargo line")
        return shlex.split(pick)

    @staticmethod
    def with_cross(argv: list[str]) -> list[str]:
        extra = ["--target", LINUX_TRIPLE, "--keep-going"]
        if "--" in argv:
            k = argv.index("--")
            return argv[:k] + extra + argv[k:]
        return argv + extra

    def cargo_argv(self, argv: list[str]) -> list[str]:
        if argv[0] != "cargo":
            raise Drift(f"not a cargo line: {argv!r}")
        return [self.cargo, *argv[1:]]

    def linux_env(self, job: Job, leg: str | None) -> dict:
        env = self.base_env()
        env.update(self.yml_env(job, leg, None, keep_rustflags=True))
        if "RUSTFLAGS" not in env:
            raise Drift(f"adapter cannot read job {job.id}: no RUSTFLAGS in the yml for a Linux row")
        sub = env.pop("CARGO_TARGET_DIR", None)
        env["CARGO_TARGET_DIR"] = str(self.linux_dir / sub) if sub else str(self.linux_dir)
        return env

    def linux_cargo(self, unit: Unit, argv: list[str], log: Path) -> tuple[int, float, str]:
        env = self.linux_env(unit.job, unit.leg)
        rc, secs = self.run(self.cargo_argv(self.with_cross(argv)), env, log, self.args.row_timeout_min)
        return rc, secs, self.read(log)

    def finish_linux(self, label: str, rc: int, secs: float, log: Path, text: str) -> Result:
        dead = self.collect_dead(label, text)
        warns = self.warning_summaries(text)
        if rc == 0 and not warns:
            return Result("PASS", rc, secs, str(log))
        detail = f"{dead} dead/unused item(s)" if dead else ""
        if warns:
            detail += (", " if detail else "") + f"{len(warns)} warning summary line(s)"
        return Result("FAIL", rc, secs, str(log), detail)

    def do_script(self, unit: Unit, log: Path) -> Result:
        steps = self.run_steps(unit.job)
        if len(steps) != 1:
            raise Drift(f"adapter cannot read job {unit.job.id}: expected one `run:` step")
        words = shlex.split(steps[0].run.strip())
        if len(words) < 2 or words[0] != "python3" or not words[1].endswith(".py"):
            raise Drift(f"adapter cannot read job {unit.job.id}: not a `python3 <script>.py` line")
        env = self.base_env()
        env.update(self.yml_env(unit.job, unit.leg, steps[0], keep_rustflags=False))
        rc, secs = self.run([sys.executable, *words[1:]], env, log, self.args.row_timeout_min)
        return Result("PASS" if rc == 0 else "FAIL", rc, secs, str(log))

    def do_linux(self, unit: Unit, log: Path) -> Result:
        jid = unit.job.id
        if jid == "feature-legs":
            return self.do_feature_legs(unit, log)
        if jid == "bench-compile":
            argv = self.single_cargo_line(unit.job, unit.leg)
            if argv[:2] != ["cargo", "bench"] or "--no-run" not in argv:
                raise Drift("adapter cannot read job bench-compile: not a `cargo bench ... --no-run` line")
            argv = ["cargo", "check", *[a for a in argv[2:] if a != "--no-run"], "--profile", "bench", "--lib", "--bins", "--benches"]
        elif jid == "test-linux":
            argv = self.release_line(unit.job, unit.leg)
            if argv[:2] != ["cargo", "test"]:
                raise Drift("adapter cannot read job test-linux: the leg's line is not `cargo test`")
            argv = ["cargo", "check", *[a for a in argv[2:] if a != "--no-fail-fast"]]
        else:
            argv = self.single_cargo_line(unit.job, unit.leg)
            if argv[1] not in ("check", "clippy"):
                raise Drift(f"adapter cannot read job {jid}: a `{unit.row.kind}` row runs `cargo check` or `cargo clippy`, not `cargo {argv[1]}`")
        rc, secs, text = self.linux_cargo(unit, argv, log)
        return self.finish_linux(unit.id, rc, secs, log, text)

    # The feature-legs derivation is the yml's own Python, run as written; only the cargo metadata
    # call gains a `--manifest-path`, so the step can run in a scratch directory instead of writing
    # its three files into the worktree.
    METADATA_CALL = '["cargo", "metadata", "--no-deps", "--format-version", "1"]'

    def derive_feature_legs(self, job: Job, workdir: Path, log: Path) -> tuple[int, list[tuple[str, str, list[str]]]]:
        steps = self.run_steps(job)
        if len(steps) != 2:
            raise Drift("adapter cannot read job feature-legs: expected two `run:` steps (derive, check)")
        check = steps[1].run
        heredocs, code = split_derive_step(steps[0].run)
        if code.count(self.METADATA_CALL) != 1:
            raise Drift("adapter cannot read job feature-legs: the derive step's `cargo metadata` call changed")
        code = code.replace(
            self.METADATA_CALL,
            '["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", %r]' % str(REPO / "Cargo.toml"),
        )
        if 'cargo check --locked -p "$pkg" --features "$feat" $scope' not in check:
            raise Drift("adapter cannot read job feature-legs: the per-leg cargo line changed")
        workdir.mkdir(parents=True, exist_ok=True)
        for name, body in heredocs.items():
            (workdir / name).write_text(body, encoding="utf-8", newline="\n")
        (workdir / "derive.py").write_text(code, encoding="utf-8", newline="\n")
        env = self.base_env()
        with self.row_logs.open(log) as out:
            out.write(b"\n$ <the derive step's python>\n")
            out.flush()
            rc = subprocess.run([sys.executable, "derive.py"], cwd=workdir, env=env, stdout=out, stderr=subprocess.STDOUT).returncode
        legs = []
        if rc == 0:
            for line in (workdir / "feature-legs.txt").read_text(encoding="utf-8").splitlines():
                if line.strip():
                    pkg, feat, *scope = line.split()
                    legs.append((pkg, feat, scope))
        return rc, legs

    def do_feature_legs(self, unit: Unit, log: Path) -> Result:
        work = (self.log_dir or Path(tempfile.gettempdir())) / "feature-legs-work"
        rc, legs = self.derive_feature_legs(unit.job, work, log)
        if rc != 0:
            return Result("FAIL", rc, 0.0, str(log), "the derive step failed")
        derived = {(p, f) for p, f, _ in legs}
        stale = sorted(CROSS_UNBUILDABLE - derived)
        if stale:
            raise Drift(f"CROSS_UNBUILDABLE names {stale}, which are not derived legs -- delete the entry")
        res = Result("PASS", 0, 0.0, str(log))
        failed = 0
        for pkg, feat, scope in legs:
            argv = ["cargo", "check", "--locked", "-p", pkg, "--features", feat, *scope]
            name = f"{pkg}/{feat}"
            # One log per leg, so a leg's diagnostics are never attributed to the leg after it.
            if (pkg, feat) in CROSS_UNBUILDABLE:
                env = self.host_env(unit.job, None, None)
                sub_log = log.with_name(f"{log.stem}.{pkg}.{feat}.native.log")
                rc2, secs = self.run(self.cargo_argv(argv), env, sub_log, self.args.row_timeout_min)
                text = self.read(sub_log)
                ok = rc2 == 0 and not self.warning_summaries(text)
                res.sub.append((name, "NOT-RUNNABLE", f"cross check needs x86_64-linux-gnu-gcc; native substitute {'PASS' if ok else 'FAIL'}"))
                res.seconds += secs
                failed += 0 if ok else 1
                continue
            sub_log = log.with_name(f"{log.stem}.{pkg}.{feat}.log")
            rc2, secs, text = self.linux_cargo(unit, argv, sub_log)
            sub = self.finish_linux(f"feature-legs[{name}]", rc2, secs, sub_log, text)
            res.seconds += secs
            res.sub.append((name, sub.status, sub.detail))
            failed += sub.status == "FAIL"
        if not legs:
            return Result("FAIL", 1, res.seconds, str(log), "no leg derived: a vacuous green")
        if failed:
            res.status, res.exit = "FAIL", 1
        res.detail = f"{len(legs)} legs, {failed} failed"
        return res

    def host_env(self, job: Job, leg: str | None, step: Step | None) -> dict:
        env = self.base_env()
        env.update(self.yml_env(job, leg, step, keep_rustflags=False))
        if "CARGO_TARGET_DIR" not in env:
            env["CARGO_TARGET_DIR"] = str(self.host_dir)
        return env

    def bash_argv(self, job: Job, step: Step, script: Path) -> list[str]:
        shell = step.shell or job.default_shell
        if shell == "bash":
            return [self.bash, "--noprofile", "--norc", "-eo", "pipefail", str(script)]
        if shell is None and job.runs_on == "ubuntu-latest":
            return [self.bash, "-e", str(script)]
        raise Drift(f"adapter cannot read job {job.id}: step shell {shell!r} on {job.runs_on}")

    def new_top_logs(self, before: set[str]) -> list[Path]:
        return [p for p in REPO.glob("*.log") if p.name not in before]

    def do_host(self, unit: Unit, log: Path, timeout_min: float) -> Result:
        job = unit.job
        jid = job.id
        deviation = ""
        if jid == "miri":
            # The deviation the row's note states, measured on this tree rather than asserted.
            sites = sorted(
                str(p.relative_to(REPO)).replace("\\", "/")
                for p in (REPO / "crates" / "boyko_ecs").rglob("*.rs")
                if "target_feature" in p.read_text(encoding="utf-8", errors="replace")
            )
            if sites:
                deviation = f"deviation: Miri interprets the AVX2 arm of {', '.join(sites)} (CI: the scalar arm)"
        if jid == "loom":
            if "--cfg loom" not in job.env.get("RUSTFLAGS", "") or len(self.run_steps(job)) != 2 or "unset RUSTFLAGS" not in self.run_steps(job)[1].run:
                raise Drift("adapter cannot read job loom: expected job RUSTFLAGS with `--cfg loom` and two steps, the second unsetting RUSTFLAGS")
        scripts = log.parent / (log.stem + ".steps")
        scripts.mkdir(parents=True, exist_ok=True)
        before = {p.name for p in REPO.glob("*.log")}
        total, rc = 0.0, 0
        try:
            for n, step in enumerate(self.run_steps(job)):
                env = self.host_env(job, unit.leg, step)
                if jid == "loom":
                    env["CARGO_TARGET_DIR"] = str(self.host_dir / "ci-mirror-loom")
                    if n == 0:
                        env["CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"] = job.env["RUSTFLAGS"]
                path = scripts / f"step{n}.sh"
                path.write_text(substitute(step.run, job, unit.leg), encoding="utf-8", newline="\n")
                rc, secs = self.run(self.bash_argv(job, step, path), env, log, timeout_min)
                total += secs
                if rc != 0:
                    break
        finally:
            # The steps `tee` their logs into the working directory, as on a throwaway CI checkout.
            # Here that is a worktree: move this row's new top-level `*.log` files beside its log.
            for p in self.new_top_logs(before):
                shutil.move(str(p), str(scripts / p.name))
        text = self.read(log)
        warns = self.warning_summaries(text)
        if rc == 0 and not warns:
            return Result("PASS", rc, total, str(log), deviation)
        detail = f"{len(warns)} warning summary line(s)" if warns else ""
        return Result("FAIL", rc, total, str(log), "; ".join(d for d in (detail, deviation) if d))

    # --- the force-alloc-panic COVERED rule -------------------------------------------------------

    def fap_rule(self) -> str | None:
        """None when `test:release` covers `force-alloc-panic`, else the reason it does not."""
        job = self.jobs.get("force-alloc-panic")
        test = self.jobs.get("test")
        if job is None or test is None:
            return "a job the rule reads is missing"
        flags = shlex.split(job.env.get("RUSTFLAGS", ""))
        top = shlex.split(self.top_env.get("RUSTFLAGS", ""))
        rest = [f for f in flags]
        try:
            k = rest.index("--cfg")
            if rest[k + 1] != "force_alloc_panic":
                return "the job's RUSTFLAGS carry another --cfg"
            del rest[k : k + 2]
        except (ValueError, IndexError):
            return "the job's RUSTFLAGS carry no `--cfg force_alloc_panic`"
        if rest != top:
            return f"the job's RUSTFLAGS minus the cfg ({' '.join(rest)}) differ from the top level ({' '.join(top)})"
        if self.single_cargo_line(job, None) != self.release_line(test, "release"):
            return "the job's cargo line is not the `test` job's release line"
        readers = []
        for p in REPO.rglob("*.rs"):
            rel = p.relative_to(REPO)
            if "target" in rel.parts[:1]:
                continue
            readers += fap_readers_in(p.read_text(encoding="utf-8", errors="replace"), rel.as_posix())
        if readers:
            return f"cfg(force_alloc_panic) is read in code: {readers[:3]}"
        return None

    def fap_rule_note(self) -> str:
        """The COVERED rule's state for a run that SKIPS the row. A source scan is cheap, so the
        default tier evaluates it too: an edit that breaks the rule shows at every merge check, not
        only hours into a --heavy run."""
        reason = self.fap_rule()
        if reason is None:
            return "COVERED rule holds (test:release covers it under --heavy)"
        return f"COVERED rule BROKEN ({reason}): under --heavy this row needs --fap-target-dir"

    # --- preflight ----------------------------------------------------------------------------------

    def preflight(self, selected: list[Unit]) -> None:
        if not os.environ.get("RUSTUP_TOOLCHAIN"):
            raise Drift("preflight: set RUSTUP_TOOLCHAIN to the installed toolchain whose `rustc -V` is ci.yml's pin (e.g. stable-x86_64-pc-windows-msvc)")
        self.cargo = shutil.which("cargo", path=os.environ.get("PATH"))
        if not self.cargo:
            raise Drift("preflight: no `cargo` on PATH")
        env = self.base_env()
        pins = {
            s.uses.split("@", 1)[1]
            for j in self.jobs.values()
            for s in j.steps
            if s.uses and s.uses.startswith("dtolnay/rust-toolchain@") and not s.uses.endswith("@nightly")
        }
        if len(pins) != 1:
            raise Drift(f"preflight: ci.yml pins {sorted(pins)}, expected exactly one stable toolchain")
        (pin,) = pins
        rustc = subprocess.run(["rustc", "-V"], env=env, capture_output=True, text=True, cwd=REPO)
        if rustc.returncode != 0 or not rustc.stdout.startswith(f"rustc {pin} "):
            raise Drift(f"preflight: `rustc -V` = {rustc.stdout.strip() or rustc.stderr.strip()!r}, ci.yml pins {pin}")
        sysroot = Path(subprocess.run(["rustc", "--print", "sysroot"], env=env, capture_output=True, text=True, cwd=REPO).stdout.strip())
        kinds = {u.row.kind for u in selected}
        if kinds & {"linux", "linux-compile-only"}:
            if self.linux_dir is None:
                raise Drift("preflight: a Linux row is selected and --linux-target-dir is not given")
            if not (sysroot / "lib" / "rustlib" / LINUX_TRIPLE / "lib").is_dir():
                raise Drift(f"preflight: the {LINUX_TRIPLE} std is not installed in {sysroot}")
        if kinds & {"host", "host-heavy"} or any(
            u.job.id == "feature-legs" for u in selected
        ):
            if self.host_dir is None:
                raise Drift("preflight: a host row is selected and neither --host-target-dir nor CARGO_TARGET_DIR is set")
        toolchain = os.environ["RUSTUP_TOOLCHAIN"]
        installed: dict[str, set[str]] = {}

        def components(tc: str) -> set[str]:
            if tc not in installed:
                out = subprocess.run(["rustup", "component", "list", "--installed", "--toolchain", tc], env=env, capture_output=True, text=True)
                installed[tc] = {c.strip().split("-x86_64-")[0] for c in out.stdout.splitlines() if c.strip()}
            return installed[tc]

        for u in selected:
            for s in u.job.steps:
                if not s.uses or not s.uses.startswith("dtolnay/rust-toolchain@"):
                    continue
                want = [c.strip() for c in str(s.with_.get("components") or "").split(",") if c.strip()]
                tc = "nightly" if s.uses.endswith("@nightly") else toolchain
                missing = [c for c in want if c not in components(tc)]
                if missing:
                    raise Drift(f"preflight: job {u.job.id} needs component(s) {missing} on {tc}")
        if kinds & {"host", "host-heavy"}:
            exec_path = subprocess.run(["git", "--exec-path"], capture_output=True, text=True).stdout.strip()
            bash = Path(exec_path).parents[2] / "bin" / "bash.exe" if exec_path else None
            if bash is None or not bash.is_file():
                raise Drift("preflight: Git Bash not found through `git --exec-path`")
            if "system32" in str(bash).lower():
                raise Drift("preflight: refusing C:\\Windows\\System32\\bash.exe (the WSL launcher)")
            self.bash = str(bash)

    # --- the run ------------------------------------------------------------------------------------

    def select(self) -> tuple[list[Unit], dict[str, str]]:
        only = {x for x in (self.args.only or "").split(",") if x}
        skip = {x for x in (self.args.skip or "").split(",") if x}
        known = {u.id for u in self.units} | {u.job.id for u in self.units}
        unknown = sorted((only | skip) - known)
        if unknown:
            raise Drift(f"--only/--skip name {unknown}, which are no job or leg of ci.yml")
        selected, skipped = [], {}
        for u in self.units:
            if only and u.id not in only and u.job.id not in only:
                skipped[u.id] = "not in --only"
            elif u.id in skip or u.job.id in skip:
                skipped[u.id] = "in --skip"
            elif u.row.kind == "host-heavy" and not self.args.heavy:
                skipped[u.id] = "host-heavy: run with --heavy"
            else:
                selected.append(u)
        return selected, skipped

    def run_all(self) -> int:
        assert self.log_dir is not None
        self.log_dir.mkdir(parents=True, exist_ok=True)
        (self.log_dir / "PID").write_text(str(os.getpid()), encoding="utf-8")
        progress = self.log_dir / "progress.tsv"
        # disk_free_gb after each row: the per-row disk cost is the difference, read without scanning
        # a target directory (a `du` over a shared target dir is what this machine must not run).
        progress.write_text("id\tkind\tstatus\texit\tseconds\tdisk_free_gb\tdetail\n", encoding="utf-8")
        selected, skipped = self.select()
        self.preflight(selected)
        sel_ids = {u.id for u in selected}
        for u in self.units:
            if u.id in skipped:
                detail = skipped[u.id]
                if u.job.id == "force-alloc-panic":
                    detail += "; " + self.fap_rule_note()
                self.results[u.id] = Result("SKIPPED", detail=detail)
                continue
            log = self.log_dir / (u.id.replace(":", "_") + ".log")
            kind = u.row.kind
            if kind == "not-runnable":
                res = Result("NOT-RUNNABLE", detail=u.row.note)
                if u.id == "test-linux:debug":
                    res.detail = self.check_debug_selection() + " " + u.row.note
            elif u.job.id == "force-alloc-panic":
                res = self.do_fap(u, log, "test:release" in sel_ids)
            else:
                why = self.guard()
                if why:
                    res = Result("FAIL", detail=f"machine guard: {why}")
                elif kind == "script":
                    res = self.do_script(u, log)
                elif kind in ("linux", "linux-compile-only"):
                    res = self.do_linux(u, log)
                else:
                    budget = self.args.heavy_row_timeout_min if kind == "host-heavy" else self.args.row_timeout_min
                    res = self.do_host(u, log, budget)
            self.results[u.id] = res
            with open(progress, "a", encoding="utf-8") as f:
                f.write(f"{u.id}\t{kind}\t{res.status}\t{res.exit}\t{res.seconds:.0f}\t{self.disk_free_gb():.1f}\t{res.detail}\n")
                for name, st, det in res.sub:
                    f.write(f"  {u.id}[{name}]\t{kind}\t{st}\t\t\t\t{det}\n")
            print(f"[ci-mirror] {u.id}: {res.status}" + (f" ({res.detail})" if res.detail else ""), flush=True)
            if self.disk_stop:
                raise Drift(f"{self.disk_stop} during row {u.id}; the run stops here (progress.tsv has the rows so far)")
        return self.report()

    def check_debug_selection(self) -> str:
        """The not-runnable debug leg's claim: its compile half is the `check` row's selection."""
        debug = self.release_line(self.jobs["test-linux"], "debug")
        check = self.single_cargo_line(self.jobs["check"], None)
        a = sorted(a for a in debug[2:] if a != "--no-fail-fast")
        b = sorted(check[2:])
        if debug[:2] != ["cargo", "test"] or check[:2] != ["cargo", "check"] or a != b:
            raise Drift("test-linux:debug's selection is no longer the `check` row's: give it a linux-compile-only row")
        return "(compile half = the `check` row's selection: verified)"

    def do_fap(self, unit: Unit, log: Path, release_selected: bool) -> Result:
        reason = self.fap_rule()
        if reason is None:
            if not release_selected:
                return Result("SKIPPED", detail="COVERED by test:release, which is not selected (--heavy)")
            return Result("COVERED", detail="by test:release: no `.rs` reads cfg(force_alloc_panic) and the RUSTFLAGS differ by that flag alone")
        if not self.args.fap_target_dir:
            return Result("FAIL", 1, detail=f"not covered ({reason}): run it with --fap-target-dir")
        if self.guard():
            return Result("FAIL", detail="machine guard")
        job = unit.job
        env = self.host_env(job, None, None)
        env["CARGO_TARGET_DIR"] = str(Path(self.args.fap_target_dir).resolve())
        # This row's CI meaning is its RUSTFLAGS, so it gets them, in its own directory.
        env["CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"] = job.env["RUSTFLAGS"]
        argv = self.single_cargo_line(job, None)
        rc, secs = self.run(self.cargo_argv(argv), env, log, self.args.heavy_row_timeout_min)
        warns = self.warning_summaries(self.read(log))
        return Result("PASS" if rc == 0 and not warns else "FAIL", rc, secs, str(log), f"not covered ({reason})")

    def report(self) -> int:
        assert self.log_dir is not None
        lines = ["id\tkind\tstatus\texit\tseconds\tlog\tdetail"]
        width = max(len(u.id) for u in self.units) + 2
        print()
        print(f"{'id':<{width}}{'kind':<20}{'status':<14}{'exit':<6}{'seconds':<9}log / detail")
        counts: dict[str, int] = {}
        for u in self.units:
            r = self.results[u.id]
            counts[r.status] = counts.get(r.status, 0) + 1
            print(f"{u.id:<{width}}{u.row.kind:<20}{r.status:<14}{str(r.exit if r.exit is not None else '-'):<6}{r.seconds:<9.0f}{Path(r.log).name if r.log else ''} {r.detail}")
            for name, st, det in r.sub:
                print(f"{'  [' + name + ']':<{width + 20}}{st:<14}{'':<15}{det}")
            lines.append(f"{u.id}\t{u.row.kind}\t{r.status}\t{r.exit}\t{r.seconds:.0f}\t{r.log}\t{r.detail}")
            for name, st, det in r.sub:
                lines.append(f"{u.id}[{name}]\t{u.row.kind}\t{st}\t\t\t\t{det}")
        (self.log_dir / "summary.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
        with open(self.log_dir / "dead_items.tsv", "w", encoding="utf-8") as f:
            f.write("leg\tfile:line\tmessage\n")
            for leg, where, msg in sorted(set(self.dead)):
                f.write(f"{leg}\t{where}\t{msg}\n")
        summary = ", ".join(f"{k} {v}" for k, v in sorted(counts.items()))
        partial = " -- PARTIAL MIRROR (a row was SKIPPED)" if counts.get("SKIPPED") else ""
        print(f"\n[ci-mirror] {summary}{partial}")
        code = 1 if counts.get("FAIL") else 0
        (self.log_dir / "DONE").write_text(str(code), encoding="utf-8")
        return code

    def list(self) -> int:
        print("JOB TABLE (tools/ci_mirror.py) against", self.ci_yml)
        for u in self.units:
            print(f"  {u.id:<28} {u.row.kind:<20} {u.job.runs_on:<16} {u.row.note[:70]}")
        if self.args.no_cargo:
            return 0
        self.cargo = shutil.which("cargo", path=os.environ.get("PATH"))
        if not self.cargo:
            raise Drift("--list derives the feature legs and needs `cargo` on PATH (or pass --no-cargo)")
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "derive.log"
            rc, legs = self.derive_feature_legs(self.jobs["feature-legs"], Path(tmp) / "work", log)
            if rc != 0:
                print(self.read(log))
                return 1
        print(f"feature legs derived by ci.yml's own step: {len(legs)}")
        for pkg, feat, scope in legs:
            tag = "  NOT-RUNNABLE cross (native substitute)" if (pkg, feat) in CROSS_UNBUILDABLE else ""
            print(f"  {pkg}/{feat} {' '.join(scope)}{tag}")
        return 0


def split_derive_step(derive: str) -> tuple[dict[str, str], str]:
    """The feature-legs derive step's two heredoc tables (name -> body) and its Python body.

    A heredoc body may be empty (`<<'EOF'` directly followed by `EOF`), so the terminator is
    matched as a whole line, not after a newline the body may not have.
    """
    heredocs = {
        m.group(1): m.group(2)
        for m in re.finditer(r"cat > ([\w.-]+) <<'EOF'\n(.*?)^EOF$", derive, re.S | re.M)
    }
    py = re.search(r"python3 - <<'PY'\n(.*?)\nPY\n?$", derive, re.S)
    if set(heredocs) != {"feature-refusals.txt", "feature-scopes.txt"} or not py:
        raise Drift(
            "adapter cannot read job feature-legs: the derive step is not two `cat <<'EOF'` tables "
            "and one `python3 - <<'PY'` body"
        )
    return heredocs, py.group(1)


_LEX_START = re.compile(r"""//|/\*|["'bcr]""")
_RAW_STR_OPEN = re.compile(r'[bc]?r(#*)"')
_CHAR_LIT = re.compile(r"b?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]{1,8}\}|[^\r\n])|[^\\'\r\n])'")
_CFG_OPEN = re.compile(r"(?<![A-Za-z0-9_])(cfg_attr|cfg)\s*(?:!\s*)?\(")
_FAP = re.compile(r"\bforce_alloc_panic\b")


def rust_code_only(text: str) -> str:
    """`text` with every comment, string literal and char literal blanked to spaces, newlines kept.

    A scan over the result sees code only, at the original line numbers. Raw strings (`r#"…"#`,
    `br"…"`, `cr"…"`), escapes, nested block comments and a char literal that holds a quote (`'"'`)
    are handled; a lifetime (`'a`) is code and stays.
    """
    out = list(text)
    n = len(text)
    i = 0
    while (m0 := _LEX_START.search(text, i)) is not None:
        i = m0.start()
        c = text[i]
        # A `b` / `c` / `r` opens a literal only as a prefix, never inside an identifier.
        prefix = c in "bcr" and not (i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"))
        end = None
        if c == "/" and text[i + 1] == "/":
            end = text.find("\n", i)
            end = n if end < 0 else end
        elif c == "/":
            depth, end = 1, i + 2
            while end < n and depth:
                if text.startswith("/*", end):
                    depth, end = depth + 1, end + 2
                elif text.startswith("*/", end):
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
        elif prefix and (raw := _RAW_STR_OPEN.match(text, i)) is not None:
            close = '"' + raw.group(1)
            k = text.find(close, raw.end())
            end = n if k < 0 else k + len(close)
        elif c == '"' or (prefix and text.startswith(('b"', 'c"'), i)):
            end = text.index('"', i) + 1
            while end < n and text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            end = min(end + 1, n)
        elif c == "'" or (c == "b" and prefix and text.startswith("b'", i)):
            lit = _CHAR_LIT.match(text, i)
            end = lit.end() if lit else None
        if end is None:
            i += 1
            continue
        for k in range(i, end):
            if out[k] != "\n":
                out[k] = " "
        i = end
    return "".join(out)


def fap_readers_in(text: str, rel: str) -> list[str]:
    """`rel:line` of every `cfg` predicate that names `force_alloc_panic`.

    A predicate is the argument of `#[cfg(…)]` or `cfg!(…)`, or the first argument of
    `#[cfg_attr(…)]`; nested ones and ones split over lines count. A comment, a string literal (an
    `#[ignore = "…"]` reason), a `--cfg` mention and a bare identifier are not readers: the COVERED
    rule is about code whose meaning the flag changes.
    """
    if "force_alloc_panic" not in text:
        return []
    code = rust_code_only(text)
    out: list[str] = []
    for m in _CFG_OPEN.finditer(code):
        depth, j, stop = 1, m.end(), None
        while j < len(code) and depth:
            ch = code[j]
            if ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
            elif ch == "," and depth == 1 and stop is None and m.group(1) == "cfg_attr":
                stop = j
            j += 1
        for f in _FAP.finditer(code, m.end(), j if stop is None else stop):
            where = f"{rel}:{code.count(chr(10), 0, f.start()) + 1}"
            if where not in out:
                out.append(where)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--linux-target-dir")
    ap.add_argument("--log-dir")
    ap.add_argument("--host-target-dir")
    ap.add_argument("--heavy", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--skip")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--no-cargo", action="store_true", help="with --list: do not derive the feature legs")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--fap-target-dir")
    ap.add_argument("--min-free-commit-gb", type=float, default=6.0)
    ap.add_argument("--min-free-disk-gb", type=float, default=15.0)
    ap.add_argument("--row-timeout-min", type=float, default=120.0)
    ap.add_argument("--heavy-row-timeout-min", type=float, default=480.0)
    ap.add_argument("--ci-yml", help="test-only: read this file instead of the repository's ci.yml")
    args = ap.parse_args()

    # The reader is proven before it is trusted with ci.yml. Exit 2, the convention of
    # scripts/check_hotpath_exceptions.py: a wrong mirror is not a red job.
    failures = self_test()
    if failures:
        print("ci-mirror SELF-TEST FAILED (ci.yml was not read):", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 2
    if args.self_test:
        print("ci-mirror self-test OK: the YAML reader, the table parser and the scanners.")
        return 0

    ci_yml = Path(args.ci_yml).resolve() if args.ci_yml else DEFAULT_CI_YML
    if not ci_yml.is_file():
        print(f"ci-mirror: {ci_yml} does not exist -- run the copy of this script that lives in the worktree to check", file=sys.stderr)
        return 2
    try:
        mirror = Mirror(args, ci_yml)
        if args.list:
            return mirror.list()
        if not args.log_dir:
            print("ci-mirror: --log-dir is required (no default writes into a worktree)", file=sys.stderr)
            return 2
        return mirror.run_all()
    except Drift as e:
        print(f"ci-mirror DRIFT: {e}", file=sys.stderr)
        if args.log_dir:
            Path(args.log_dir).mkdir(parents=True, exist_ok=True)
            (Path(args.log_dir) / "DONE").write_text("2", encoding="utf-8")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
