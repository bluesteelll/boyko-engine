#!/usr/bin/env python3
"""UG-16 (plan gate, rung B2 baseline): the linked section sizes of `boyko_demo`, recorded per rung.

What it measures
----------------
`llvm-size -A -d` (System V format, decimal) on the linked `boyko_demo` executable built by
`cargo build --release -p boyko_demo` under `[profile.release]` (`lto = "fat"`). It records `.text`,
`.rdata` (the PE name of the plan's `.rodata`) and `Total`, and prints every section.

Why the System V format: the default Berkeley format folds `.rdata` into its `data` column on a PE
image (`text data bss dec hex`, where data = .rdata + .data + .pdata + .fptable + .tls + _RDATA +
.rsrc + .reloc) — measured with the tool itself on this host, rung B2's `ug16_llvm_size_format.txt`.
Section sizes are unpadded virtual sizes, so a few added bytes are visible.

The band
--------
Against the TSV's last row: `.text` or `.rdata` above +1 % is exit 2, HOLD — the rung waits for the
OWNER's written acceptance; no agent accepts it (plan 03, UG-16). `Total` is recorded, not banded.
The first row of a record is written with `--baseline` (rung B2), which is legal only on an empty
record.

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
* the output does not parse, or `.text` is 0.

What a row identifies
---------------------
`head`, `dirty` (any tracked modification or untracked file outside `docs/` and `*.md` — files that
cannot reach the image) and `tree_id` (sha256 of HEAD, `git diff HEAD` over those tracked paths and
the bytes of those untracked files, 12 hex digits), so a row taken on an uncommitted tree says so.
Record AFTER the rung's code commit and commit the row separately, and the row reads `dirty=0`.

Usage
-----
    python scripts/ug16_linked_size.py --image D:/wt/_targets/ui-msvc/release/boyko_demo.exe \
        [--record docs/measurements/ug16-linked-size.tsv --rung B2 [--baseline]] \
        [--compare docs/measurements/ug16-linked-size.tsv] [--llvm-size PATH] \
        [--toolchain stable-x86_64-pc-windows-msvc] [--note TEXT]

Exit: 0 PASS / recorded, 1 RED, 2 HOLD (band exceeded; owner's written acceptance required).
No timing of any kind is taken; binary size is not timing.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
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
    if out.get(".text", 0) == 0 or "Total" not in out:
        red(f"could not parse a non-zero `.text` and a `Total` from llvm-size -A -d:\n{p.stdout}")
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
    _, _, deps = text.partition(": ")
    placeholder = "\u0001"
    deps = deps.replace("\\ ", placeholder)
    paths = [Path(tok.replace(placeholder, " ")) for tok in deps.split()]
    if not paths:
        red(f"dep-info `{d}` lists no input")
    return paths


def check_image(image: Path) -> float:
    if not image.is_file():
        red(f"image `{image}` does not exist")
    if "release" not in image.resolve().parts:
        red(f"image `{image}` has no `release` path segment — UG-16 measures the `profile.release` build")
    mtime = image.stat().st_mtime
    inputs = dep_info_inputs(image)
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
    print(f"      inputs {len(local)} source file(s) in this tree per dep-info; newest {newest.relative_to(repo).as_posix()} is not newer than the image")
    return mtime


def read_rows(tsv: Path) -> list[dict[str, str]]:
    if not tsv.exists():
        return []
    rows = []
    header: list[str] | None = None
    for line in tsv.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        cells = line.split("\t")
        if header is None:
            header = cells
            if tuple(header) != COLUMNS:
                red(f"{tsv}: header {header} is not {list(COLUMNS)}")
            continue
        rows.append(dict(zip(header, cells)))
    return rows


def main() -> int:
    # The messages carry non-ASCII punctuation; a redirected Windows console would re-encode them.
    sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[union-attr]
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", required=True, type=Path)
    ap.add_argument("--llvm-size", type=Path)
    ap.add_argument("--toolchain", default="stable-x86_64-pc-windows-msvc")
    ap.add_argument("--record", type=Path, help="append a row to this TSV")
    ap.add_argument("--compare", type=Path, help="compare against this TSV's last row without recording")
    ap.add_argument("--rung", help="the rung the recorded row belongs to")
    ap.add_argument("--baseline", action="store_true", help="first row of an empty record (rung B2)")
    ap.add_argument("--note", default="")
    a = ap.parse_args()

    tool = a.llvm_size or discover_tool(a.toolchain)
    if not tool.is_file():
        red(f"llvm-size not found at `{tool}` (rustup component `llvm-tools`; or pass --llvm-size)")
    version = tool_version(tool)
    image = a.image
    mtime = check_image(image)
    secs = sections(tool, image)
    head, dirty, tree_id = tree_identity()

    print(f"UG-16 image {image}")
    print(f"      tool  {tool} (LLVM {version}), toolchain {a.toolchain}")
    print(f"      tree  head={head} dirty={dirty} tree_id={tree_id}")
    print(f"{'section':<12}{'size':>12}")
    for name, size in secs.items():
        print(f"{name:<12}{size:>12}")

    tsv = a.record or a.compare
    rows = read_rows(tsv) if tsv else []
    verdict = "PASS"
    if rows:
        prev = rows[-1]
        print(f"against the last row (rung {prev['rung']}, head {prev['head'][:12]}):")
        for name, col in ((".text", "text"), (".rdata", "rdata"), ("Total", "total")):
            old = int(prev[col])
            new = secs.get(name, 0)
            pct = (new - old) / old * 100 if old else float("inf")
            mark = ""
            if name in BANDED and old and (new - old) / old > BAND:
                verdict = "HOLD"
                mark = f"  > +{BAND:.0%} band"
            print(f"  {name:<8} {old:>12} -> {new:>12}  delta {new - old:+d} B ({pct:+.4f} %){mark}")
    elif tsv:
        print(f"no previous row in {tsv}: nothing to compare against")

    if a.record:
        if not a.rung:
            red("--record needs --rung")
        if a.baseline and rows:
            red(f"--baseline is legal only on an empty record; {a.record} has {len(rows)} row(s)")
        if not a.baseline and not rows:
            red(f"{a.record} has no rows: the first row is written with --baseline")
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
            "rdata": str(secs.get(".rdata", 0)),
            "total": str(secs["Total"]),
            "sections": ";".join(f"{k}={v}" for k, v in secs.items() if k != "Total"),
            "note": a.note.replace("\t", " "),
        }
        new_file = not a.record.exists()
        with a.record.open("a", encoding="utf-8", newline="\n") as f:
            if new_file:
                f.write("# UG-16 (plan gate, rung B2): linked section sizes of boyko_demo, profile.release (fat LTO),\n")
                f.write("# llvm-size -A -d. One row per rung; written by scripts/ug16_linked_size.py --record.\n")
                f.write("# Band: .text or .rdata above +1 % vs the previous row is HOLD for the owner's written acceptance.\n")
                f.write("\t".join(COLUMNS) + "\n")
            f.write("\t".join(row[c] for c in COLUMNS) + "\n")
        print(f"UG-16 RECORDED: rung {a.rung} -> {a.record}")
        return 0

    print(f"UG-16 {verdict}")
    return 2 if verdict == "HOLD" else 0


if __name__ == "__main__":
    sys.exit(main())
