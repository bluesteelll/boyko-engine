#!/usr/bin/env python3
"""Gate every hot-path type-ban exception behind a written, reviewed justification.

`clippy.toml`'s `disallowed-types` mechanically bans `HashMap`/`HashSet`/`Mutex`/`RwLock`/
`Rc`/`RefCell` (CLAUDE.md "Forbidden on the hot path", Principles 1/4), and
`[workspace.lints.clippy] disallowed_types = "deny"` makes that ban fire in the editor and in a
plain local `cargo clippy` rather than only under CI's `-D warnings`.

That leaves exactly ONE escape hatch, measured empirically during the 2026-07 audit: the lint is
silenced by an `#[allow]`, and an `#[allow]` on a `mod` (or a crate-level `#![allow]`) silences an
entire subtree invisibly. Clippy cannot police its own suppressions, so this script does:

  1. BLANKET SUPPRESSION IS FORBIDDEN in production code. An exception must sit on the single item
     that needs it, never on a `mod` and never at crate root.
  2. EVERY exception must be enumerated in `docs/HOT-PATH-EXCEPTIONS.md`, one row per site, with a
     frequency class drawn from a closed vocabulary of provably-cold classes. Writing `per-frame`
     is not an option the vocabulary offers — a hot site has to be FIXED, not documented.
  3. The row count per file must match the allow count per file, so adding a new exception to an
     already-registered file still fails until its own row is written.

Why the count-match instead of line numbers: line numbers churn on every edit above the site and
would make this gate a nuisance that gets disabled. The pair (file, count) is stable under
unrelated edits and still catches every addition and removal.

The precedent this exists for: `trigger.rs`'s `TRIGGER_IDS` carried a hand-written comment
claiming "Cold (registration-only ... never on the trigger hot path)". It was false — the map was
locked on every relation link/unlink in the per-frame command drain. A prose comment nobody
re-reads is not a control. A row in a file the reviewer diffs is.

Usage:
    python scripts/check_hotpath_exceptions.py [--list]

Exit 0 = registry and sources agree. Exit 1 = drift (message names the file and the delta).
`--list` prints the sites the sources actually contain, in registry-row format, to seed the doc.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY = REPO / "docs" / "HOT-PATH-EXCEPTIONS.md"

# Production = anything that ships in a library/binary target. Integration tests, benches and
# examples are free to model with std collections (a proptest oracle SHOULD use a HashMap — that
# is how it stays an independent check on the boyko-native structure it verifies).
NON_PRODUCTION = ("/tests/", "/benches/", "/examples/", "/target/")

ALLOW_RE = re.compile(r"#!?\[(allow|expect)\(\s*clippy::disallowed_types")
INNER_RE = re.compile(r"#!\[(allow|expect)\(\s*clippy::disallowed_types")
MOD_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+")

# Closed vocabulary. Each class is a claim about how often the banned type is TOUCHED — not about
# how often it is written. A memoized registry whose memo lookup itself takes the lock is
# `per-frame`, and `per-frame` is deliberately absent: there is no way to register a hot site.
COLD_CLASSES = {
    "once-per-process",  # a single construction/read for the lifetime of the process
    "once-per-type",     # a rust#22991 TypeId mint whose per-call fast path is lock-free
    "load-time",         # asset/scene load, outside the frame loop
    "boot",              # device/window/pool construction before the first frame
    "shutdown",          # teardown after the last frame
    "debug-only",        # inside #[cfg(debug_assertions)] or a debug_assert! invariant check
    "codegen-tool",      # offline generator behind a default-off feature; never in a shipped binary
    # A production-resident helper whose ONLY callers are test/bench targets, established by an
    # exhaustive workspace grep rather than by a cfg. Weaker than the others by construction: the
    # item still compiles into the library, so a future caller can make it hot without tripping
    # this gate. Use it only when the item cannot be moved behind `#[cfg(test)]`, and say in the
    # justification WHICH targets call it so the next auditor can re-run the same grep.
    "test-harness",
    # A `RefCell` (never a lock or a map) on a `!Send + !Sync` owner, used purely as the borrow
    # gate around an operation that dominates it by orders of magnitude — a `vkCreateBuffer`, a
    # heap allocation. It cannot contend, so Principle 4's lock-free rule is not what is at
    # stake; the honest justification is a cost argument, and the class exists so that argument
    # has to be MADE rather than smuggled in under `boot`. Requires naming the guarded operation.
    "alloc-guarded",
}

BLANKET_HEADING = "## Blanket exemptions"
ROWS_HEADING = "## Exceptions"

def split_row(line: str) -> list[str] | None:
    """Splits a markdown table row into stripped cells, or `None` if it is not one.

    Deliberately a splitter and not a regex over cell CONTENT: a cell legitimately holds more
    than one backticked token (`intern()` / `resolve()`, `Mutex`, `Condvar`), and a regex tight
    enough to reject junk also rejected those. The one content rule is that a cell may not
    contain a literal `|`.
    """
    s = line.strip()
    if not s.startswith("|") or not s.endswith("|"):
        return None
    return [c.strip() for c in s[1:-1].split("|")]


MOD_DECL_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;")
CFG_TEST_RE = re.compile(r"^\s*#\[cfg\(test\)\]")
PATH_ATTR_RE = re.compile(r'^\s*#\[path\s*=\s*"(?P<path>[^"]+)"\s*\]')


def _all_sources() -> list[Path]:
    roots = [REPO / "crates", REPO / "src"]
    out: list[Path] = []
    for root in roots:
        if not root.is_dir():
            continue
        for p in root.rglob("*.rs"):
            if any(part in p.as_posix() for part in NON_PRODUCTION):
                continue
            out.append(p)
    return sorted(out)


def test_only_files(sources: list[Path]) -> set[Path]:
    """Files pulled in exclusively by a `#[cfg(test)] mod foo;` declaration.

    These live under `src/` (so the directory filter misses them) but never compile into a
    library or binary target — `boyko_physics/src/resources_tests.rs` and
    `boyko_rhi_vulkan/src/compute/tests.rs` are the two in-tree cases. Resolved from the
    DECLARATION rather than from a filename convention, so a production file that merely happens
    to be named `tests.rs` is not silently exempted.
    """
    out: set[Path] = set()
    for path in sources:
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        for i, line in enumerate(lines):
            if not CFG_TEST_RE.match(line):
                continue
            # `#[path = "..."]` may sit between the cfg and the `mod` (boyko_physics does this to
            # keep a 3k-line module's tests in their own file under the ORIGINAL module name).
            explicit: str | None = None
            for nxt in lines[i + 1 : i + 4]:
                pm = PATH_ATTR_RE.match(nxt)
                if pm:
                    explicit = pm.group("path")
                    continue
                m = MOD_DECL_RE.match(nxt)
                if m:
                    name = m.group("name")
                    cands = (
                        [path.parent / explicit]
                        if explicit
                        else [
                            path.parent / f"{name}.rs",
                            path.parent / name / "mod.rs",
                            # `foo.rs` + `foo/` sibling-directory layout: `mod tests;` inside
                            # `compute.rs` resolves to `compute/tests.rs`.
                            path.parent / path.stem / f"{name}.rs",
                        ]
                    )
                    for cand in cands:
                        if cand.is_file():
                            out.add(cand.resolve())
                    break
                if nxt.strip() and not nxt.strip().startswith(("#[", "//")):
                    break
    return out


def cfg_test_spans(lines: list[str]) -> list[tuple[int, int]]:
    """Inclusive line-index spans covered by an inline `#[cfg(test)] mod ... { }` block.

    A `#[cfg(test)]` module inside a production file is test code and may model with std
    collections freely — a proptest oracle built on a `HashMap` is exactly how the boyko-native
    structure under test gets an INDEPENDENT check. Without this, the gate fires on 20 such
    modules and becomes the kind of nuisance that gets switched off.

    Brace counting is naive about braces inside strings and comments. The failure mode is
    over-reach to end-of-file, and a `#[cfg(test)]` module is conventionally last, so an
    over-reach costs nothing here. It is deliberately NOT a general Rust parser.
    """
    spans: list[tuple[int, int]] = []
    n, i = len(lines), 0
    while i < n:
        if not CFG_TEST_RE.match(lines[i]):
            i += 1
            continue
        j = i + 1
        while j < n and j < i + 4 and not re.match(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+", lines[j]):
            j += 1
        if j >= n or not re.match(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+", lines[j]):
            i += 1
            continue
        if lines[j].rstrip().endswith(";"):  # out-of-line decl — handled by test_only_files
            i = j + 1
            continue
        depth, k, opened = 0, j, False
        while k < n:
            depth += lines[k].count("{") - lines[k].count("}")
            if "{" in lines[k]:
                opened = True
            if opened and depth <= 0:
                break
            k += 1
        spans.append((i, min(k, n - 1)))
        i = k + 1
    return spans


def production_sources() -> list[Path]:
    srcs = _all_sources()
    skip = test_only_files(srcs)
    return [p for p in srcs if p.resolve() not in skip]


def scan_sources() -> tuple[dict[str, int], list[tuple[str, int, str]]]:
    """Returns (rel_path -> per-item allow count) and the blanket-suppression sites.

    Blanket sites are NOT counted per-item: one `#![allow]` covers a whole module, so it is
    reconciled against the registry's "Blanket exemptions" section instead — one reviewed
    decision, one row. Unregistered blanket sites are the failure the gate exists for.
    """
    counts: dict[str, int] = {}
    blanket: list[tuple[str, int, str]] = []
    for path in production_sources():
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        rel = path.relative_to(REPO).as_posix()
        spans = cfg_test_spans(lines)
        n = 0
        for i, line in enumerate(lines):
            if not ALLOW_RE.search(line):
                continue
            if any(lo <= i <= hi for lo, hi in spans):
                continue  # inside an inline #[cfg(test)] module — not production
            if INNER_RE.search(line):
                blanket.append((rel, i + 1, "crate/module-level `#![allow]`"))
                continue
            # Look ahead past further attributes, doc comments and blank lines to the item itself.
            is_mod = False
            for nxt in lines[i + 1 : i + 12]:
                s = nxt.strip()
                if not s or s.startswith(("#[", "///", "//!", "//")):
                    continue
                is_mod = bool(MOD_RE.match(nxt))
                break
            if is_mod:
                blanket.append((rel, i + 1, "`#[allow]` on a `mod`"))
                continue
            n += 1
        if n:
            counts[rel] = n
    return counts, blanket


def parse_registry() -> tuple[dict[str, int], set[str], list[str]]:
    if not REGISTRY.exists():
        return {}, set(), [f"missing registry: {REGISTRY.relative_to(REPO).as_posix()}"]
    counts: dict[str, int] = {}
    blanket: set[str] = set()
    errors: list[str] = []
    # Only the two registry sections are parsed. The prose above them contains illustrative
    # tables (the fixed-violations summary, the class vocabulary) whose rows would otherwise be
    # read as exceptions.
    section: str | None = None
    for i, line in enumerate(REGISTRY.read_text(encoding="utf-8").splitlines(), 1):
        if line.startswith("## "):
            heading = line.strip()
            section = (
                "blanket" if heading == BLANKET_HEADING else "rows" if heading == ROWS_HEADING else None
            )
            continue
        if section is None:
            continue
        cells = split_row(line)
        # Header and separator rows of the section's own table.
        if cells is None or not cells[0].startswith("`"):
            continue
        want = 4 if section == "blanket" else 5
        if len(cells) != want:
            shape = (
                "`file` | `scope` | class | why"
                if section == "blanket"
                else "`file` | `symbol` | `type` | class | why"
            )
            errors.append(f"{REGISTRY.name}:{i}: malformed row (need {want} cells: {shape})")
            continue
        file = cells[0].strip("`")
        cls, why = cells[-2], cells[-1]
        if cls not in COLD_CLASSES:
            errors.append(
                f"{REGISTRY.name}:{i}: frequency class `{cls}` is not a cold class. "
                f"Allowed: {', '.join(sorted(COLD_CLASSES))}. A hot site must be fixed, not registered."
            )
        if len(why) < 40:
            errors.append(
                f"{REGISTRY.name}:{i}: justification is {len(why)} chars — too short to be an "
                f"analysis. State what makes it cold AND why a boyko-native structure cannot serve."
            )
        if section == "blanket":
            blanket.add(file)
        else:
            counts[file] = counts.get(file, 0) + 1
    return counts, blanket, errors


def main() -> int:
    src_counts, src_blanket = scan_sources()

    if "--list" in sys.argv:
        for rel, n in sorted(src_counts.items()):
            for _ in range(n):
                print(f"| `{rel}` | `?` | `?` | once-per-type | ? |")
        for rel, line, kind in src_blanket:
            print(f"BLANKET | `{rel}` | `?` | codegen-tool | ? |   (line {line}, {kind})")
        return 0

    reg_counts, reg_blanket, errors = parse_registry()

    for rel, line, kind in src_blanket:
        if rel not in reg_blanket:
            errors.append(
                f"{rel}:{line}: unregistered {kind} suppresses the ban for an entire subtree. "
                f"Either move it onto the single item that needs it, or — if the whole module is "
                f"genuinely off the engine path — add a row under '{BLANKET_HEADING}' in "
                f"docs/HOT-PATH-EXCEPTIONS.md."
            )
    for rel in sorted(reg_blanket):
        if rel not in {r for r, _, _ in src_blanket}:
            errors.append(f"{rel}: registered as a blanket exemption but the file has no `#![allow]` left — delete the stale row.")

    for rel in sorted(set(src_counts) | set(reg_counts)):
        got, want = src_counts.get(rel, 0), reg_counts.get(rel, 0)
        if got == want:
            continue
        if want == 0:
            errors.append(
                f"{rel}: {got} unregistered exception(s). Add a row per site to "
                f"docs/HOT-PATH-EXCEPTIONS.md justifying why the banned type is cold there."
            )
        elif got == 0:
            errors.append(
                f"{rel}: registry lists {want} exception(s) but the file has none left. "
                f"Delete the stale row(s) — a registry that outlives its sites stops being read."
            )
        else:
            errors.append(f"{rel}: {got} exception(s) in source vs {want} row(s) in the registry.")

    if errors:
        print("hot-path exception gate FAILED:\n", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        print(
            f"\n{sum(src_counts.values())} exception(s) across {len(src_counts)} file(s).",
            file=sys.stderr,
        )
        return 1

    print(f"hot-path exception gate OK: {sum(src_counts.values())} registered exception(s) across {len(src_counts)} file(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
