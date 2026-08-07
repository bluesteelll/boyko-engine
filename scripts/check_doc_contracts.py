#!/usr/bin/env python3
"""Keep a split planning corpus from re-creating the seam defect it was split to avoid.

`docs/PROFILING-SYSTEM-PLAN.md` and `docs/LOGGING-SYSTEM-PLAN.md` were written separately and
reviewed separately, twice each. Both passed. The first reader to hold them side by side found
them asserting CONTRADICTORY FACTS about the same object: profiling justified moving its ABI into
`boyko_utils` because that crate has zero dependencies, while logging stated flatly that
`boyko_utils` depends on `boyko_log`. Both cannot be true. Underneath that, each document had
independently invented the same four primitives -- a per-thread lane index, an `rdtsc`
calibration, a never-freeing lane allocator, a loss accounting -- with incompatible semantics.

Nothing caught it for three review rounds, because nothing was WATCHING THE BOUNDARY. Every gate
in both documents pointed inward.

Splitting the corpus into ~22 small files makes each piece reviewable, and makes that failure
MORE likely, not less: two documents can disagree in one way, twenty-two can disagree in many.
So the split ships with this gate.

Each file declares, in a `CONTRACT` block directly under its H1, the capabilities it PROVIDES and
the ones it ASSUMES from elsewhere. That turns "do these documents agree?" -- a question that
needs a reviewer to have read all of them at once -- into a graph the machine walks.

  <!-- CONTRACT
  provides: substrate/lane-registry     one owner per capability; another file may assume it
  exports: profiling/public-api         terminal: a human consumes it, no file need assume it
  assumes:  seam/clock-ownership        must be provided (or exported) by exactly one other file
  -->

Checks, and the real defect each one catches:

  1. DANGLING     every `assumes` resolves to a `provides`/`exports` somewhere.
                  Catches: a piece leaning on a decision that was edited away in its owner.
  2. AMBIGUOUS    each capability id is provided by exactly ONE file.
                  Catches: two pieces both claiming to own the clock -- the S3/S4 defect, which
                  is how one worker ended up as lane 5 to one subsystem and lane 37 to the other.
  3. SELF         no file assumes what it provides.
                  Catches: a piece that looks connected but is talking to itself.
  4. CYCLE        the assumption graph is acyclic.
                  Catches THE S2 DEFECT DIRECTLY. "Profiling assumes utils is the bottom" plus
                  "logging assumes log is below utils" is a cycle, and a cycle is exactly what a
                  reader cannot hold in their head -- which is why three rounds missed it.
  5. ORPHAN       every `provides` is assumed by someone. A capability nobody consumes is either
                  dead text or a missing `exports:`; both are worth one line to resolve.
  6. INDEX        every id appears in the corpus README's map, so the index cannot rot silently.

Why capability ids and not file/anchor links: anchors churn on every heading edit, so an
anchor-based check becomes a nuisance and gets disabled -- the same reasoning that made
`check_hotpath_exceptions.py` match on (file, count) rather than line numbers. A capability id is
a NAME for a decision; it survives the section being rewritten, and it stops surviving exactly
when the decision is deleted, which is when the alarm should fire.

This gate refuses to be vacuous. If the corpus directory is absent it exits 1 rather than
passing quietly: a gate with no subject that reports success is the failure mode this campaign has
now caught nine times. Wire it into CI in the same commit that lands the corpus, not before.

Usage:
    python scripts/check_doc_contracts.py            # the gate
    python scripts/check_doc_contracts.py --list     # print the map, in README-map format
    python scripts/check_doc_contracts.py --graph    # print the assumption edges, for eyeballing

Exit 0 = the corpus is internally consistent. Exit 1 = a named violation, or no corpus at all.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "docs" / "diagnostics"
README = CORPUS / "README.md"

# The contract block must sit in an HTML comment so it renders as nothing in a Markdown viewer,
# and must be the first such comment in the file so a reader meets it before the prose.
BLOCK_RE = re.compile(r"<!--\s*CONTRACT\s*(.*?)-->", re.DOTALL)
LINE_RE = re.compile(r"^\s*(provides|exports|assumes)\s*:\s*(\S+)\s*(?:#.*)?$")

# `<area>/<slug>`. The area is part of the id so ownership is legible without opening a file --
# an `assumes: substrate/...` in a profiling piece is visibly a cross-area edge.
AREAS = ("substrate", "profiling", "logging", "seam")
ID_RE = re.compile(r"^(" + "|".join(AREAS) + r")/[a-z0-9]+(?:-[a-z0-9]+)*$")


class Piece:
    """One corpus file's declared contract."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.rel = path.relative_to(REPO).as_posix()
        self.provides: list[str] = []
        self.exports: list[str] = []
        self.assumes: list[str] = []

    @property
    def owns(self) -> list[str]:
        return self.provides + self.exports


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)


def parse(path: Path, errors: list[str]) -> Piece | None:
    text = path.read_text(encoding="utf-8-sig")
    m = BLOCK_RE.search(text)
    if not m:
        errors.append(
            f"{path.relative_to(REPO).as_posix()}: no CONTRACT block. Every corpus file declares "
            f"what it provides and assumes, even if the answer is a single `exports:` line."
        )
        return None

    piece = Piece(path)
    for lineno, raw in enumerate(m.group(1).splitlines(), start=1):
        if not raw.strip():
            continue
        lm = LINE_RE.match(raw)
        if not lm:
            errors.append(f"{piece.rel}: unparsable CONTRACT line {lineno}: {raw.strip()!r}")
            continue
        kind, cap = lm.group(1), lm.group(2)
        if not ID_RE.match(cap):
            errors.append(
                f"{piece.rel}: bad capability id {cap!r}. Expected `<area>/<kebab-slug>` with "
                f"area in {AREAS}."
            )
            continue
        getattr(piece, kind).append(cap)
    return piece


def check(pieces: list[Piece], errors: list[str]) -> None:
    # --- 2. AMBIGUOUS: exactly one owner per capability -----------------------------------
    owner: dict[str, str] = {}
    for p in pieces:
        for cap in p.owns:
            if cap in owner:
                errors.append(
                    f"{cap}: owned by BOTH {owner[cap]} and {p.rel}. Exactly one file owns a "
                    f"capability -- two owners is how the same worker became lane 5 to one "
                    f"subsystem and lane 37 to the other."
                )
            else:
                owner[cap] = p.rel

    # --- 1. DANGLING + 3. SELF ------------------------------------------------------------
    for p in pieces:
        for cap in p.assumes:
            if cap not in owner:
                errors.append(
                    f"{p.rel}: assumes {cap}, which no file provides. Either the owner deleted "
                    f"it, or it was never written."
                )
            elif cap in p.owns:
                errors.append(f"{p.rel}: assumes {cap}, which it also provides.")

    # --- 5. ORPHAN ------------------------------------------------------------------------
    assumed = {cap for p in pieces for cap in p.assumes}
    for p in pieces:
        for cap in p.provides:
            if cap not in assumed:
                errors.append(
                    f"{p.rel}: provides {cap}, which no file assumes. If a human is the consumer, "
                    f"declare it `exports:` instead; if nothing consumes it, it is dead text."
                )

    # --- 4. CYCLE -------------------------------------------------------------------------
    # Edge: consumer -> owner-of-the-capability-it-assumes. A cycle means two pieces each rest on
    # the other's conclusion, which is exactly the shape of the contradiction three review rounds
    # failed to see.
    graph: dict[str, set[str]] = {p.rel: set() for p in pieces}
    for p in pieces:
        for cap in p.assumes:
            tgt = owner.get(cap)
            if tgt and tgt != p.rel:
                graph[p.rel].add(tgt)

    WHITE, GREY, BLACK = 0, 1, 2
    colour = {n: WHITE for n in graph}
    stack: list[str] = []

    def visit(node: str) -> None:
        colour[node] = GREY
        stack.append(node)
        for nxt in sorted(graph[node]):
            if colour[nxt] == GREY:
                cut = stack[stack.index(nxt):]
                errors.append(
                    "assumption CYCLE: " + " -> ".join(cut + [nxt]) + ". Each of these rests on "
                    "the next one's conclusion; no reader can check either in isolation."
                )
            elif colour[nxt] == WHITE:
                visit(nxt)
        stack.pop()
        colour[node] = BLACK

    for node in sorted(graph):
        if colour[node] == WHITE:
            visit(node)

    # --- 7. UNDECLARED CROSS-AREA DEPENDENCY ----------------------------------------------
    # Checks 1-6 all reason about the DECLARED graph, and a declared graph is only as honest as
    # its declarations. Measured on the first real corpus: 43 capabilities and 132 declared
    # edges, and the acyclicity check was green -- while `substrate/00-GOAL.md` declared ZERO
    # edges and discussed SEAM.md and a profiling file in its prose. Those undeclared references
    # point UP (substrate -> seam -> profiling); had they been declared, check 4 would have found
    # a cycle. So check 4's green was bought by silence, which is the very shape of gate this
    # corpus exists to eliminate.
    #
    # A file that DISCUSSES another area's file depends on it. Same-area siblings are exempt:
    # within one subsystem, "see the ladder file" is navigation between parts of one argument,
    # and its cycle risk is contained. Cross-area is not navigation -- it is a claim about
    # someone else's decision, and it must be declared so the graph can be checked.
    def area_of(p: Piece) -> str:
        parts = p.path.relative_to(CORPUS).parts
        return parts[0] if len(parts) > 1 else "_root"

    # Match on the AREA-QUALIFIED path, never the bare basename. Measured: this corpus has four
    # colliding basenames (`05-LADDER-GATES.md` in three areas, plus `06-DISPOSITIONS.md`,
    # `04-GAME-FACING.md` and `00-GOAL-TARGETS.md` in two each). A basename-keyed dict collapses
    # nine files into four entries, which produced BOTH failure directions at once: a file citing
    # its own same-area sibling was reported as an undeclared cross-area dependency (three false
    # positives, which earned three fabricated `assumes:` declarations before this was caught),
    # and the five collapsed-away files were never checked as targets at all — the gate was
    # silently blind on more files than it was wrong about.
    for p in pieces:
        body = BLOCK_RE.sub("", p.path.read_text(encoding="utf-8-sig"))
        declared = {owner[c] for c in p.assumes if c in owner}
        mine = area_of(p)
        for other in pieces:
            if other is p or area_of(other) == mine:
                continue
            qualified = other.path.relative_to(CORPUS).as_posix()
            if qualified not in body:
                continue
            if other.rel not in declared:
                errors.append(
                    f"{p.rel}: discusses {other.rel} in prose but declares no capability from "
                    f"it. A cross-area reference IS a dependency; declaring it is what lets "
                    f"check 4 see the edge. (Undeclared upward edges are how an acyclic-looking "
                    f"graph hides a cycle.)"
                )

    # --- 6. INDEX -------------------------------------------------------------------------
    if not README.exists():
        errors.append(f"{README.relative_to(REPO).as_posix()} is missing: the corpus has no index.")
    else:
        index_text = README.read_text(encoding="utf-8-sig")
        for cap in sorted(owner):
            if cap not in index_text:
                errors.append(
                    f"{cap} ({owner[cap]}) is absent from the README map. The index is how a "
                    f"reader finds an owner without grepping; it rots the moment it may lag."
                )


def main() -> int:
    args = set(sys.argv[1:])

    if not CORPUS.is_dir():
        fail(
            f"{CORPUS.relative_to(REPO).as_posix()} does not exist, so this gate has no subject. "
            f"It exits 1 rather than passing quietly -- a green gate with nothing to check is the "
            f"defect this corpus was split to make visible. Wire this script into CI in the same "
            f"commit that lands the corpus."
        )
        return 1

    files = sorted(CORPUS.rglob("*.md"))
    if not files:
        fail(f"{CORPUS.relative_to(REPO).as_posix()} contains no Markdown files.")
        return 1

    errors: list[str] = []
    # The README is the corpus INDEX, not a piece of the design: it owns no capability and assumes
    # none. Check 6 reads it; it does not declare a contract of its own.
    pieces = [p for p in (parse(f, errors) for f in files if f != README) if p is not None]

    if pieces:
        check(pieces, errors)

    if "--list" in args:
        print("| capability | owner | assumed by |")
        print("|---|---|---|")
        owner = {c: p.rel for p in pieces for c in p.owns}
        for cap in sorted(owner):
            users = sorted(p.rel for p in pieces if cap in p.assumes)
            # ASCII only: this repo's Windows console is not UTF-8, and a mangled byte in a
            # table a human pastes into the README is a defect the README then carries.
            print(f"| `{cap}` | `{owner[cap]}` | {', '.join(f'`{u}`' for u in users) or '(exported)'} |")

    if "--graph" in args:
        owner = {c: p.rel for p in pieces for c in p.owns}
        for p in sorted(pieces, key=lambda x: x.rel):
            for cap in sorted(p.assumes):
                print(f"{p.rel} -> {owner.get(cap, '??')}   [{cap}]")

    if errors:
        for e in errors:
            fail(e)
        print(
            f"\n{len(errors)} contract violation(s) across {len(files)} file(s).", file=sys.stderr
        )
        return 1

    print(f"OK: {len(files)} corpus files, contracts consistent.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
