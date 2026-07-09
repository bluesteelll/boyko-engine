#!/usr/bin/env python
"""PreToolUse gate for Bash: nudge 'graphify-first' before a raw search command.

Improvement over the former inline one-liner: WORD-LEVEL matching. The old
`case "$CMD" in *grep*|*ag\\ *|...` fired on substrings ('tag ', 'flag ', 'manage ',
'find' anywhere in a path). This inspects the LEADING command of each pipeline /
'&&' / ';' stage, so it fires only when a stage actually invokes a search tool.
Fail-open: any error prints nothing and exits 0.
"""
import json
import os
import re
import sys

SEARCH = {"grep", "egrep", "fgrep", "rg", "ripgrep", "find", "fd", "fdfind", "ack", "ag"}

REMINDER = (
    "MANDATORY: graphify-out/graph.json exists. You MUST run `graphify query \"<question>\"` before "
    "grepping raw files. Only grep after graphify has oriented you, or to modify/debug specific lines. "
    "If graphify returns off-target or empty results, fall back to grep once - do NOT retry graphify."
)

_SKIP = {"sudo", "command", "time", "env", "nice", "nohup", "\\"}


def leading_cmd(seg):
    """First real command token of a stage, skipping env-assigns and wrappers."""
    for tok in seg.strip().split():
        if "=" in tok and not tok.startswith("-"):
            continue  # FOO=bar prefix
        if tok in _SKIP:
            continue
        return tok.rsplit("/", 1)[-1]  # /usr/bin/grep -> grep
    return ""


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
        cmd = str(d.get("tool_input", d).get("command") or "")
        if not cmd.strip():
            return
        for seg in re.split(r"\|\||&&|[|;]", cmd):
            if leading_cmd(seg) in SEARCH:
                out = {"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": REMINDER}}
                sys.stdout.write(json.dumps(out))
                return
    except Exception:
        return


if __name__ == "__main__":
    main()
