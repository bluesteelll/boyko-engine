#!/usr/bin/env python3
"""Semantic search over this tree: semble recall, cross-encoder ranking.

WHY THIS EXISTS. `semble` alone answers the easy half of the "meaning in prose and
comments" question and misses the hard half, and the reason was measured rather than
guessed (2026-09-07, six questions with ripgrep-verified ground truth, phrased without
the target's own vocabulary; harness kept in the session scratchpad):

    first stage only          MRR 0.219   hits@10 2/6
    + ms-marco-MiniLM-L-6-v2  MRR 0.385   hits@10 5/6      80 MB,   ~4 s/query
    + bge-reranker-base       MRR 0.602   hits@10 5/6     1.04 GB,  ~17 s/query

The failure was RANKING, not chunking or recall: a near-verbatim query retrieves the
target chunk at rank 1 with score ~0.0197, while a paraphrase of the same question
leaves it outside the top 5 at ~0.009. semble's own `rerank` is a lexical heuristic
(file boost, identifier boost, path penalties over an RRF fusion with BM25) and was
already ON in those measurements, so the cross-encoder's gain is on top of it, not
instead of it. What a static embedding model cannot do is read the query and the chunk
TOGETHER — it sums token vectors with no attention — so the bridge from "cannot be
tuned" to "no constant threshold can certify it" is out of its reach by construction.
That is why a bigger static model does not help, and this was tested too:
`potion-retrieval-32M` and `potion-base-32M` both score WORSE than the default
`potion-code-16M-v2` and lose a code target out of the top 40 entirely, and merging all
three candidate pools recovers no recall the default did not already have.

LIMITS, so the next reader does not over-trust the numbers. Six questions is a small
set and three of them target the same claim written in three places, so the ordering
BETWEEN rerankers is suggestive only; the size of the rerank effect (2/6 -> 5/6) is not.
One question in the set is missed by every configuration despite its chunk being
indexed, which is a recall failure the reranker cannot repair. Read the top few hits
rather than trusting position 1.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

# The default model is deliberate: it measured best on both MRR and recall. Do not
# "upgrade" it without re-running the harness.
MODEL = os.environ.get("SEMBLE_MODEL_NAME", "minishlab/potion-code-16M-v2")
CACHE = os.environ.get("SEMBLE_CACHE_LOCATION", r"D:\tmp\semble-cache")
RERANKER = os.environ.get("SEMSEARCH_RERANKER", "Xenova/ms-marco-MiniLM-L-6-v2")

# semble hardcodes `_DESIRED_CHUNK_LENGTH_CHARS = 750` (~190 tokens) with a
# `# TODO: make this configurable`, against a corpus whose rationale blocks average ~2,200
# chars. At 750 a block is split into ~4 fragments, and semble's own ranker then multiplies
# the 2nd fragment from a file by 0.5, the 3rd by 0.25 - so fragmentation is punished twice.
# Swept over 27 questions: 750 is the WORST of {750, 1500, 3000, 4500, 6000} on every metric.
CHUNK_CHARS = int(os.environ.get("SEMSEARCH_CHUNK_CHARS", "4500"))

# Rank-fusion weight: 1.0 = first stage only, 0.0 = cross-encoder only. The interior point
# beats BOTH extremes, which is why this is not a switch. Measured at budget 4500:
# hits@10 = 11 (a=1), 14 (a=0), 16 (a=0.5).
ALPHA = float(os.environ.get("SEMSEARCH_ALPHA", "0.5"))
RRF_K = 60

# There is no env var or CLI flag for the chunk budget, so run semble IN-PROCESS with the
# module constant patched. This is deliberately NOT an edit to site-packages: a patched
# install is silently reverted by the next `pip install -U semble`, and the same trap has
# already cost this project a month of stale index. Patching at call time survives upgrades.
# All three read sites resolve the constant when called (two through deferred imports), and
# semble's own cache compares `metadata["chunk_size"]` against it - so changing the budget
# self-invalidates the index and no manual cache clearing is needed.
_RUNNER = (
    "import semble.chunking.chunking as _c;"
    "_c._DESIRED_CHUNK_LENGTH_CHARS={n};"
    "import semble.cli as _m;_m.main()"
)

# Below this, semble's top hit is noise rather than an answer: a near-verbatim match
# lands at ~0.0197 and paraphrases that genuinely hit land well above the field.
WEAK_SCORE = 0.012


def first_stage(question: str, tree: str, pool: int) -> list[dict]:
    env = dict(os.environ)
    env["SEMBLE_MODEL_NAME"] = MODEL
    env["SEMBLE_CACHE_LOCATION"] = CACHE
    p = subprocess.run(
        [sys.executable, "-c", _RUNNER.format(n=CHUNK_CHARS),
         "search", question, tree, "--content", "all", "--top-k", str(pool)],
        capture_output=True, text=True, env=env, encoding="utf-8", errors="replace",
    )
    raw = p.stdout or ""
    i = raw.find('{"query"')
    if i < 0:
        sys.stderr.write((p.stderr or raw)[:600] + "\n")
        raise SystemExit("semble produced no result object; is it installed?")
    return json.loads(raw[i:])["results"]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("question")
    ap.add_argument("tree", nargs="?", default=".")
    ap.add_argument("-k", "--top-k", type=int, default=8, help="results to print")
    ap.add_argument("--pool", type=int, default=40, help="candidates to rerank")
    ap.add_argument("--no-rerank", action="store_true", help="first stage only")
    ap.add_argument("--snippet", type=int, default=3, help="lines of context to print")
    a = ap.parse_args()

    cands = first_stage(a.question, a.tree, a.pool)
    if not cands:
        print("no candidates")
        return 1

    top_raw = max(c["score"] for c in cands)
    ranked = cands

    for i, c in enumerate(cands, 1):
        c["stage1_rank"] = i

    if not a.no_rerank:
        from fastembed.rerank.cross_encoder import TextCrossEncoder

        ce = TextCrossEncoder(model_name=RERANKER)
        scores = list(ce.rerank(a.question, [c["content"] for c in cands]))
        ce_pos = {i: p for p, i in
                  enumerate(sorted(range(len(cands)), key=lambda i: -scores[i]), 1)}
        # Fuse over RANKS, not raw scores: semble returns ~0.01 similarities and the
        # cross-encoder returns logits that go negative, so the two share no scale.
        # Replacing the first-stage order outright was measured to move one target from
        # rank 38 to 2 while pushing a rank-1 target down to 9 - the documented
        # "phantom hits" failure of pointwise rerankers at depth.
        fused = sorted(
            range(len(cands)),
            key=lambda i: -(ALPHA / (RRF_K + i + 1) + (1 - ALPHA) / (RRF_K + ce_pos[i])),
        )
        ranked = [dict(cands[i], rerank_score=scores[i]) for i in fused]

    if top_raw < WEAK_SCORE:
        print(f"# NOTE first-stage top score {top_raw:.4f} < {WEAK_SCORE} — nothing")
        print("# really matched the query. Treat these as weak leads, and prefer a")
        print("# ripgrep guess: a rare word from the question often beats this.")

    # `was=` makes the reranker's redistribution visible. It is NOT a free improvement:
    # measured, it lifted three questions into the top 2 and pushed the one the first
    # stage had already nailed from rank 1 down to 9. When `was=` is small and the new
    # position is large, the first-stage order was the better one for that query — run
    # again with --no-rerank rather than trusting either order blindly.
    for n, r in enumerate(ranked[: a.top_k], 1):
        rs = r.get("rerank_score")
        tail = f"  rerank={rs:+.2f}  was=#{r['stage1_rank']}" if rs is not None else ""
        print(f"#{n} {r['file_path']}:{r['start_line']}-{r['end_line']}"
              f"  recall={r['score']:.4f}{tail}")
        if a.snippet:
            for line in r["content"].splitlines()[: a.snippet]:
                print("    " + line.rstrip()[:160])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
