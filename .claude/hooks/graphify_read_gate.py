#!/usr/bin/env python
"""PreToolUse gate for Read|Glob: nudge 'graphify-first' before reading SOURCE files.

Improvements over the former inline one-liner:
  * Excludes config/meta trees graphify never indexes (.claude/, graphify-out/,
    target/, book/, .git/) so it stops firing on agent-config .md reads.
  * Appends a 'graphify-miss -> grep, do not retry' escape note.
Fail-open: any error prints nothing and exits 0 (never blocks a tool call).
"""
import json
import os
import sys

REMINDER = (
    "MANDATORY: graphify-out/graph.json exists. You MUST run graphify before reading source files. "
    'Use: `graphify query "<question>"` (scoped subgraph), `graphify explain "<concept>"`, or '
    '`graphify path "<A>" "<B>"`. Only read raw files after graphify has oriented you, or to '
    "modify/debug specific lines. If graphify returns off-target or empty results, fall back to "
    "Grep/Read once - do NOT retry graphify more than once. This rule applies to subagents too - "
    "include it in every subagent prompt involving code exploration."
)

# Source/doc extensions graphify's AST chunker actually indexes. (Shader .hlsl is NOT
# indexed yet -- omitted on purpose so the nudge is not a lie; re-add when chunking lands.)
EXTS = (
    ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".rs", ".java", ".rb", ".c", ".h",
    ".cpp", ".hpp", ".cc", ".cs", ".kt", ".swift", ".php", ".scala", ".lua", ".sh",
    ".md", ".rst", ".txt", ".mdx",
)

# Not project source -> graphify does not index these, so the reminder is pure noise.
EXCLUDE = ("graphify-out/", ".claude/", "target/", "book/", ".git/", "node_modules/")


def main():
    try:
        raw = sys.stdin.buffer.read().decode("utf-8", "replace")
        d = json.loads(raw) if raw.strip() else {}
    except Exception:
        return
    try:
        root = os.environ.get("CLAUDE_PROJECT_DIR") or "."
        if not os.path.exists(os.path.join(root, "graphify-out", "graph.json")):
            return
        t = d.get("tool_input", d)
        s = " ".join(str(t.get(k) or "") for k in ("file_path", "pattern", "path"))
        s = s.lower().replace("\\", "/")
        if not s.strip():
            return
        if any(x in s for x in EXCLUDE):
            return
        if not any(e in s for e in EXTS):
            return
        out = {"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": REMINDER}}
        sys.stdout.write(json.dumps(out))
    except Exception:
        return


if __name__ == "__main__":
    main()
