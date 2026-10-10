#!/usr/bin/env python3
"""UG-16 (plan gate, rung B2 baseline): the linked section sizes of the UG-16 images, recorded per rung.

What it measures
----------------
`llvm-size -A -d` (System V format, decimal) on a linked executable built under `[profile.release]`
(`lto = "fat"`) and the target's `.cargo/config.toml` rustflags, never `RUSTFLAGS`. It records
`.text`, `.rdata` (the PE name of the plan's `.rodata`) and `Total`, and prints every section. The
image is taken by path, and each image has its own record, banded against its own previous row
(plan 03, UG-16 and its §1 note). The header of a new record names the image's file stem and the
`--rung` of its first row.

* `boyko_demo`, from rung B2: `cargo build --release --locked -p boyko_demo` (B2 built it before the
  lock was tracked, without `--locked`), image `<target>/release/boyko_demo.exe`, record
  `docs/measurements/ug16-linked-size.tsv`. It links only `boyko_ecs`, `boyko_threadpool`,
  `boyko_diag` and `boyko_log` (plus `eframe`/`wgpu`), so it cannot see a render, physics, UI or app
  rung.
* the playground, from rung B2b: `cargo build --release --locked -v -p boyko-app --example
  playground --message-format=json > <target>/release/examples/playground.json`, image
  `<target>/release/examples/playground.exe`, passed with `--artifacts` naming that JSON stream (see
  the stale guard below), record `docs/measurements/ug16-linked-size-playground.tsv`. An example
  inherits `boyko-app`'s dev-dependency graph and features (B3 PC-40), so its row's `--note` carries
  the `--cfg feature=…` set of the final `rustc --crate-name playground` line that `-v` prints on
  stderr (the stream's `features` for the example says the same), and a size move traced to a
  dev-dependency change is named as that, not as the rung's.

Why the System V format: the default Berkeley format folds `.rdata` into its `data` column on a PE
image (`text data bss dec hex`, where data = .rdata + .data + .pdata + .fptable + .tls + _RDATA +
.rsrc + .reloc) — measured with the tool itself on this host, rung B2's `ug16_llvm_size_format.txt`.
Section sizes are unpadded virtual sizes, so a few added bytes are visible.

The band
--------
Against the TSV's last row: `.text` or `.rdata` above +1 % is exit 2, HOLD — the rung waits for the
OWNER's written acceptance; no agent accepts it (plan 03, UG-16). `Total` is recorded, not banded.
The first row of a record is written with `--baseline` (rung B2 for `boyko_demo`, B2b for the
playground), which is legal only on an empty record.

Fail-closed (exit 1, RED — never a skip)
----------------------------------------
* the tool is absent (discovered through `rustc +<toolchain> --print sysroot` and `rustc -vV`'s host:
  `<sysroot>/lib/rustlib/<host>/bin/llvm-size[.exe]`, from rustup's `llvm-tools` component;
  `--llvm-size` overrides);
* the image is absent, or its path has no `release` segment;
* the image is STALE or FOREIGN, judged from its own input list — cargo's dep-info file beside it
  (`boyko_demo.d`), which names every source the link consumed: RED if any of those files in this
  tree is newer than the image (an edit or a checkout after the build), if any crate source it names
  lies in another checkout, or if the dep-info file is missing. A newer input is always curable by
  `cargo build`, because cargo's own fingerprint tracks the same list (a commit-time or tracked-file
  mtime test is not: committing, or editing a test, would red an image cargo will never relink);
* the dep-info beside the image is rustc's own rather than cargo's (it names itself as a target), and
  `--artifacts` is not given. An MSVC executable carries no `-C extra-filename`, because its PDB path
  is embedded in it, so rustc links an example straight into `examples/`; cargo has nothing to
  uplift and writes no dep-info of its own, and the `<name>.d` there lists the example's file alone.
  Its guard would pass over every dependency's source;
* under `--artifacts` (the build's `--message-format=json` stdout, for such an image): the stream is
  older than the image or is not JSON, no `compiler-artifact` in it has the image as its
  `executable`, or a unit in it has no rustc dep-info beside its artifacts. The input list is then
  the union of every unit's rustc dep-info, a workspace member's paths taken against this tree, and
  the stale and foreign tests above apply to it. Not read: an in-tree build script's
  `rerun-if-changed` paths other than its own sources (the only one today, `boyko_diag`'s, names
  `build.rs` alone);
* the output does not parse, or `.text`, `.rdata` or `Total` is missing or 0 (a banded section read
  as 0 would disable the band on the next row);
* the invocation is not exactly one of `--record` / `--compare` (a run that compares nothing must
  not print PASS), or `--rung` / `--baseline` / `--note` are given without `--record`; a usage error
  is exit 1, never argparse's 2, which is this script's HOLD;
* the record path resolves outside the tree this script lives in. A relative `--record` /
  `--compare` is taken against THIS tree, never the working directory: the agent harness resets the
  working directory to another checkout after every call, so a cwd-relative record would be read
  from — or appended to — a tree that is not the one measured;
* under `--compare`, the record is absent or has no data row: a comparison against nothing is not
  a PASS (the first row is written with `--record --baseline`);
* the record, wherever it exists, has no header row, a header other than the columns below, a data
  row with a different cell count, or a banded cell that is not a positive integer.

What a row identifies
---------------------
`head`, `dirty` (any tracked modification or untracked file outside `docs/` and `*.md` — files that
cannot reach the image) and `tree_id` (sha256 of HEAD, `git diff HEAD` over those tracked paths and
the bytes of those untracked files, 12 hex digits), so a row taken on an uncommitted tree says so.
Record AFTER the rung's code commit and commit the row separately, and the row reads `dirty=0`.

Usage
-----
    python scripts/ug16_linked_size.py --image D:/wt/_targets/ui-msvc/release/boyko_demo.exe \
        (--record docs/measurements/ug16-linked-size.tsv --rung B2 [--baseline] [--note TEXT]
         | --compare docs/measurements/ug16-linked-size.tsv) \
        [--llvm-size PATH] [--toolchain stable-x86_64-pc-windows-msvc]
    python scripts/ug16_linked_size.py --image D:/wt/_targets/ui-msvc/release/examples/playground.exe \
        --artifacts D:/wt/_targets/ui-msvc/release/examples/playground.json \
        (--record docs/measurements/ug16-linked-size-playground.tsv --rung B2b [--baseline] [--note TEXT]
         | --compare docs/measurements/ug16-linked-size-playground.tsv) \
        [--llvm-size PATH] [--toolchain stable-x86_64-pc-windows-msvc]

Exit: 0 PASS / recorded, 1 RED, 2 HOLD (band exceeded; owner's written acceptance required).
No timing of any kind is taken; binary size is not timing.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

REPO = Path(__file__).resolve().parent.parent
BAND = 0.01
BANDED = (".text", ".rdata")
COLUMNS = (
    "rung", "date_utc", "head", "dirty", "tree_id", "toolchain", "llvm_size_version", "image",
    "image_mtime_utc", "text", "rdata", "total", "sections", "note",
)


def red(msg: str) -> NoReturn:
    print(f"UG-16 RED: {msg}")
    sys.exit(1)


class Parser(argparse.ArgumentParser):
    """argparse exits 2 on a usage error, and 2 is this script's HOLD: a caller reading the exit
    code alone would take a mistyped invocation for a band excursion awaiting the owner."""

    def error(self, message: str) -> NoReturn:
        self.print_usage(sys.stdout)
        red(f"usage: {message}")


def run(cmd: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    # Bytes in, decoded here: `text=True` decodes with the console code page on Windows.
    p = subprocess.run(cmd, cwd=cwd, capture_output=True)
    p.stdout = p.stdout.decode("utf-8", errors="replace")  # type: ignore[assignment]
    p.stderr = p.stderr.decode("utf-8", errors="replace")  # type: ignore[assignment]
    return p


def discover_tool(toolchain: str) -> Path:
    sysroot = run(["rustc", f"+{toolchain}", "--print", "sysroot"])
    if sysroot.returncode != 0:
        red(f"`rustc +{toolchain} --print sysroot` failed: {sysroot.stderr.strip()}")
    vv = run(["rustc", f"+{toolchain}", "-vV"])
    host = next((l.split(":", 1)[1].strip() for l in vv.stdout.splitlines() if l.startswith("host:")), None)
    if not host:
        red(f"`rustc +{toolchain} -vV` printed no host line")
    exe = "llvm-size.exe" if os.name == "nt" else "llvm-size"
    return Path(sysroot.stdout.strip()) / "lib" / "rustlib" / host / "bin" / exe


def tool_version(tool: Path) -> str:
    v = run([str(tool), "--version"])
    m = re.search(r"LLVM version (\S+)", v.stdout)
    if v.returncode != 0 or not m:
        red(f"`{tool} --version` gave no LLVM version (exit {v.returncode})")
    return m.group(1)


def sections(tool: Path, image: Path) -> dict[str, int]:
    p = run([str(tool), "-A", "-d", str(image)])
    if p.returncode != 0:
        red(f"llvm-size exited {p.returncode}: {p.stderr.strip()}")
    out: dict[str, int] = {}
    for line in p.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].isdigit() and (parts[0].startswith((".", "_")) or parts[0] == "Total"):
            out[parts[0]] = int(parts[1])
    if any(out.get(name, 0) == 0 for name in (*BANDED, "Total")):
        red(f"could not parse a non-zero {', '.join(f'`{n}`' for n in (*BANDED, 'Total'))} from llvm-size -A -d:\n{p.stdout}")
    return out


def git(*args: str) -> str:
    p = run(["git", *args], cwd=REPO)
    if p.returncode != 0:
        red(f"git {' '.join(args)} failed: {p.stderr.strip()}")
    return p.stdout


def image_relevant(path: str) -> bool:
    return not (path.startswith("docs/") or path.endswith(".md"))


def tree_identity() -> tuple[str, int, str]:
    """`(HEAD, dirty, tree_id)`. Untracked files count: a new source file reaches the image
    without ever appearing in `git diff HEAD`, so its bytes are hashed directly."""
    head = git("rev-parse", "HEAD").strip()
    tracked: list[str] = []
    untracked: list[str] = []
    for line in git("status", "--porcelain", "--untracked-files=all").splitlines():
        if len(line) < 4:
            continue
        path = line[3:].split(" -> ")[-1].strip('"')
        if not image_relevant(path):
            continue
        (untracked if line.startswith("??") else tracked).append(path)
    h = hashlib.sha256(head.encode())
    if tracked:
        h.update(git("diff", "HEAD", "--", *tracked).encode("utf-8", errors="replace"))
    for u in sorted(untracked):
        h.update(u.encode("utf-8"))
        f = REPO / u
        if f.is_file():
            h.update(f.read_bytes())
    return head, int(bool(tracked or untracked)), h.hexdigest()[:12]


def dep_info_inputs(image: Path) -> list[Path]:
    """The image's own input list: cargo's dep-info file beside it (`boyko_demo.exe` ->
    `boyko_demo.d`), which names every source file of every crate the link consumed."""
    d = image.with_suffix(".d")
    if not d.is_file():
        red(f"no dep-info `{d}` beside the image — cannot tell which sources it was built from")
    text = d.read_text(encoding="utf-8", errors="replace")
    # Make-style: `target: dep dep ...`, a backslash-newline continues a line, and a space inside a
    # path is written as backslash-space.
    text = text.replace("\\\n", " ")
    target, _, deps = text.partition(": ")
    if Path(target.strip()).resolve() == d.resolve():
        red(
            f"dep-info `{d}` is rustc's own (it names itself as a target): it lists this unit's files, not its "
            "dependencies', because cargo writes no dep-info for an image it links in place (an MSVC example) — "
            "pass --artifacts with the build's --message-format=json stream"
        )
    placeholder = "\u0001"
    deps = deps.replace("\\ ", placeholder)
    paths = [Path(tok.replace(placeholder, " ")) for tok in deps.split()]
    if not paths:
        red(f"dep-info `{d}` lists no input")
    return paths


def rule_inputs(d: Path) -> list[Path]:
    """Every prerequisite of every rule in a rustc dep-info file (`target: dep ...`, `dep:` rules with
    none, `#` comments). A relative path is a workspace member's, which cargo passes to rustc
    relative to the workspace root: this tree."""
    placeholder = "\u0001"
    out: list[Path] = []
    for line in d.read_text(encoding="utf-8", errors="replace").replace("\\\n", " ").splitlines():
        if line.startswith("#"):
            continue
        _, sep, deps = line.partition(": ")
        if sep:
            for tok in deps.replace("\\ ", placeholder).split():
                p = Path(tok.replace(placeholder, " "))
                out.append(p if p.is_absolute() else REPO / p)
    return out


def unit_dep_info(msg: dict) -> Path | None:
    """The rustc dep-info beside a `compiler-artifact`'s files: `libX-<hash>.rlib` -> `X-<hash>.d`,
    `X-<hash>.dll` -> `X-<hash>.d`, `examples/X.exe` -> `examples/X.d`; a build script's artifact is its
    uplifted `build-script-build[.exe]`, and rustc wrote `build_script_build-<hash>.d` beside it, the
    hash being its directory's suffix."""
    name = msg["target"]["name"].replace("-", "_")
    for f in map(Path, msg["filenames"]):
        if "custom-build" in msg["target"]["kind"]:
            d = f.parent / f"{name}-{f.parent.name.rsplit('-', 1)[-1]}.d"
        else:
            stem = f.stem[3:] if f.suffix in (".rlib", ".rmeta") and f.stem.startswith("lib") else f.stem
            d = f.with_name(stem + ".d")
        if d.is_file():
            return d
    return None


def artifact_inputs(stream: Path, image: Path) -> tuple[list[Path], int]:
    """The input list of an image cargo writes no dep-info for, from the build's own
    `--message-format=json` stream: the union of the rustc dep-info of every unit in it, and the
    number of units."""
    if not stream.is_file():
        red(f"--artifacts `{stream}` does not exist")
    if stream.stat().st_mtime < image.stat().st_mtime:
        red(f"--artifacts `{stream}` is older than the image — it is not the stream of the build that linked it")
    units = []
    for lineno, line in enumerate(stream.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        if not line.strip():
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            red(f"{stream}:{lineno}: not a JSON message — --artifacts takes cargo's --message-format=json stdout alone")
        if msg.get("reason") == "compiler-artifact":
            units.append(msg)
    if not any(u.get("executable") and Path(u["executable"]).resolve() == image.resolve() for u in units):
        red(f"no compiler-artifact in `{stream}` has `{image}` as its executable — not this image's build")
    paths: list[Path] = []
    for u in units:
        d = unit_dep_info(u)
        if d is None:
            red(f"unit `{u['target']['name']}` in `{stream}` has no rustc dep-info beside {u['filenames']}")
        paths += rule_inputs(d)
    # rustc lists a unit's sources once per output it names (the `.d` itself, the rlib, the rmeta).
    return list(dict.fromkeys(paths)), len(units)


def check_image(image: Path, artifacts: Path | None) -> float:
    if not image.is_file():
        red(f"image `{image}` does not exist")
    if "release" not in image.resolve().parts:
        red(f"image `{image}` has no `release` path segment — UG-16 measures the `profile.release` build")
    mtime = image.stat().st_mtime
    if artifacts:
        inputs, n = artifact_inputs(artifacts, image)
        via = f"the rustc dep-info of {n} unit(s) in --artifacts"
    else:
        inputs, via = dep_info_inputs(image), "dep-info"
    repo = REPO.resolve()
    local = []
    for p in inputs:
        rp = p.resolve()
        if rp.is_relative_to(repo):
            local.append(rp)
        elif "crates" in rp.parts and "registry" not in rp.parts and "git" not in rp.parts:
            red(f"image input `{p}` is outside this tree ({repo}) — the image was built from another checkout")
    if not local:
        red("no image input lies in this tree — the image was built from another checkout")
    newest = max(local, key=lambda f: f.stat().st_mtime if f.exists() else float("inf"))
    if not newest.exists() or newest.stat().st_mtime > mtime:
        red(f"image is older than its input `{newest.relative_to(repo).as_posix()}` (or the input is gone) — stale; rebuild")
    print(f"      inputs {len(local)} source file(s) in this tree per {via}; newest {newest.relative_to(repo).as_posix()} is not newer than the image")
    return mtime


def record_path(p: Path, flag: str) -> Path:
    """The record is a file of the tree this script measures: a relative path is taken against that
    tree, not the working directory, and a path resolving anywhere else is RED."""
    repo = REPO.resolve()
    rp = (p if p.is_absolute() else repo / p).resolve()
    if not rp.is_relative_to(repo):
        red(f"{flag} `{p}` resolves to `{rp}`, outside this tree ({repo}) — the record belongs to the tree it measures")
    if rp.exists() and not rp.is_file():
        red(f"{flag} `{rp}` exists and is not a file")
    return rp


def read_rows(tsv: Path) -> list[dict[str, str]]:
    """The record's data rows; `[]` only for a file that does not exist yet (which only
    `--record --baseline` may then create). A file that exists is held to its shape."""
    if not tsv.exists():
        return []
    rows = []
    header: list[str] | None = None
    for lineno, line in enumerate(tsv.read_text(encoding="utf-8").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        cells = line.split("\t")
        if header is None:
            header = cells
            if tuple(header) != COLUMNS:
                red(f"{tsv}:{lineno}: header {header} is not {list(COLUMNS)}")
            continue
        if len(cells) != len(COLUMNS):
            red(f"{tsv}:{lineno}: {len(cells)} cell(s), the header has {len(COLUMNS)}")
        row = dict(zip(header, cells))
        for col in ("text", "rdata", "total"):
            if not row[col].isdecimal() or int(row[col]) == 0:
                red(f"{tsv}:{lineno}: `{col}` is `{row[col]}`, not a positive integer — the band cannot be applied against it")
        rows.append(row)
    if header is None:
        red(f"{tsv} exists but has no header row")
    return rows


def main() -> int:
    # The messages carry non-ASCII punctuation; a redirected Windows console would re-encode them.
    sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    ap = Parser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", required=True, type=Path)
    ap.add_argument("--llvm-size", type=Path)
    ap.add_argument("--artifacts", type=Path, help="the build's --message-format=json stdout, for an image cargo writes no dep-info for")
    ap.add_argument("--toolchain", default="stable-x86_64-pc-windows-msvc")
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--record", type=Path, help="append a row to this TSV (a file of this tree)")
    mode.add_argument("--compare", type=Path, help="compare against this TSV's last row without recording")
    ap.add_argument("--rung", help="the rung the recorded row belongs to (--record only)")
    ap.add_argument("--baseline", action="store_true", help="first row of an empty record, rung B2 / B2b (--record only)")
    ap.add_argument("--note", default="", help="free text for the recorded row (--record only)")
    a = ap.parse_args()

    # The record is settled before anything is measured, so no verdict is ever printed over a
    # comparison that had nothing to compare against.
    if a.compare:
        if a.rung or a.baseline or a.note:
            ap.error("--rung / --baseline / --note apply only to --record")
        tsv = record_path(a.compare, "--compare")
        if not tsv.is_file():
            red(f"--compare record `{tsv}` does not exist — nothing to compare against, so the band cannot be applied")
        rows = read_rows(tsv)
        if not rows:
            red(
                f"--compare record `{tsv}` has no data row — nothing to compare against, so the band cannot be "
                "applied (the first row is written with --record --baseline)"
            )
    else:
        if not a.rung:
            red("--record needs --rung")
        tsv = record_path(a.record, "--record")
        rows = read_rows(tsv)
        if a.baseline and rows:
            red(f"--baseline is legal only on an empty record; {tsv} has {len(rows)} row(s)")
        if not a.baseline and not rows:
            red(f"{tsv} has no rows: the first row is written with --baseline")

    tool = a.llvm_size or discover_tool(a.toolchain)
    if not tool.is_file():
        red(f"llvm-size not found at `{tool}` (rustup component `llvm-tools`; or pass --llvm-size)")
    version = tool_version(tool)
    image = a.image
    mtime = check_image(image, a.artifacts)
    secs = sections(tool, image)
    head, dirty, tree_id = tree_identity()

    print(f"UG-16 image {image}")
    print(f"      tool  {tool} (LLVM {version}), toolchain {a.toolchain}")
    print(f"      tree  head={head} dirty={dirty} tree_id={tree_id}")
    print(f"      record {tsv}")
    print(f"{'section':<12}{'size':>12}")
    for name, size in secs.items():
        print(f"{name:<12}{size:>12}")

    verdict = "PASS"
    if rows:
        prev = rows[-1]
        print(f"against the last row (rung {prev['rung']}, head {prev['head'][:12]}):")
        for name, col in ((".text", "text"), (".rdata", "rdata"), ("Total", "total")):
            # Both sides are positive: `read_rows` and `sections` RED on a zero or missing cell.
            old = int(prev[col])
            new = secs[name]
            mark = ""
            if name in BANDED and (new - old) / old > BAND:
                verdict = "HOLD"
                mark = f"  > +{BAND:.0%} band"
            print(f"  {name:<8} {old:>12} -> {new:>12}  delta {new - old:+d} B ({(new - old) / old * 100:+.4f} %){mark}")
    else:
        # Reachable only under `--record --baseline`: `--compare` REDs above on an empty record.
        print(f"no previous row in {tsv}: this is the baseline row")

    if a.record:
        if verdict == "HOLD":
            print("UG-16 HOLD: above the +1 % band — the row is NOT recorded; the owner's written acceptance is required")
            return 2
        row = {
            "rung": a.rung,
            "date_utc": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "head": head,
            "dirty": str(dirty),
            "tree_id": tree_id,
            "toolchain": a.toolchain,
            "llvm_size_version": version,
            "image": image.as_posix(),
            "image_mtime_utc": dt.datetime.fromtimestamp(mtime, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "text": str(secs[".text"]),
            "rdata": str(secs[".rdata"]),
            "total": str(secs["Total"]),
            "sections": ";".join(f"{k}={v}" for k, v in secs.items() if k != "Total"),
            "note": a.note.replace("\t", " "),
        }
        new_file = not tsv.exists()
        with tsv.open("a", encoding="utf-8", newline="\n") as f:
            if new_file:
                # `boyko_demo`'s baseline (`--rung B2`) wrote this line with both names literal.
                f.write(f"# UG-16 (plan gate, rung {a.rung}): linked section sizes of {image.stem}, profile.release (fat LTO),\n")
                f.write("# llvm-size -A -d. One row per rung; written by scripts/ug16_linked_size.py --record.\n")
                f.write("# Band: .text or .rdata above +1 % vs the previous row is HOLD for the owner's written acceptance.\n")
                f.write("\t".join(COLUMNS) + "\n")
            f.write("\t".join(row[c] for c in COLUMNS) + "\n")
        print(f"UG-16 RECORDED: rung {a.rung} -> {tsv}")
        return 0

    print(f"UG-16 {verdict}")
    return 2 if verdict == "HOLD" else 0


if __name__ == "__main__":
    sys.exit(main())
