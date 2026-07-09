#!/usr/bin/env python
"""UserPromptSubmit backstop: remind the orchestrator to CLARIFY ambiguous action
requests before touching files.

Subagents structurally cannot ask the user (no AskUserQuestion / Plan Mode), so
enforcement of "always clarify an ambiguous plan/impl before acting" lives at the
orchestrator level. This hook injects a reminder (never blocks) when a prompt is
action-oriented (imperative verb) yet carries NO concrete scope (no file / path /
symbol) and is short enough that the detail is unlikely to be present.

Deliberately conservative to bound false positives: fires only on the AND of
(imperative) AND (no locator) AND (len < 500). It REMINDS; it never blocks.
"""
import json
import re
import sys

# English + Russian imperative openers commonly used to request code changes.
IMPERATIVES = (
    "implement", "fix", "add", "refactor", "change", "build", "write", "create",
    "remove", "delete", "update", "rename", "migrate", "optimize", "optimise",
    "rewrite", "wire", "hook up", "make it", "port ",
    "сделай", "сделаем", "исправь", "почини", "добавь", "переделай", "реализуй",
    "измени", "напиши", "создай", "удали", "обнови", "переименуй", "оптимизируй",
    "отрефактори", "перепиши", "внедри", "подключи",
)

# Signals that the prompt already names concrete scope -> no nudge needed.
_FILE_RE = re.compile(r"[\w./-]+\.(rs|md|toml|ps1|hlsl|json|py|txt|lock|yaml|yml|sh|cfg)\b")
_PATH_RE = re.compile(r"(crates/|docs/|src/|tests/|\.claude/|book/|scripts/|target/)")
_SYMBOL_HINTS = ("`", "#[", "fn ", "struct ", "impl ", "trait ", "mod ", "::", "line ", "l:")

REMINDER = (
    "REMINDER (project rule -- clarify before acting): this request looks action-oriented "
    "but may not pin concrete scope (no file / path / symbol detected). Before any Write/Edit: "
    "confirm you know WHICH files/functions and the intended behavior. If anything is ambiguous, "
    "ask a clarifying question (AskUserQuestion) or enter Plan Mode FIRST -- do not guess. "
    "Changes touching >=3 files, or not describable in one sentence, should go through Plan Mode. "
    "This is a reminder, not a block."
)


def has_locator(p):
    if _FILE_RE.search(p) or _PATH_RE.search(p):
        return True
    return any(h in p for h in _SYMBOL_HINTS)


def main():
    try:
        raw = sys.stdin.buffer.read().decode("utf-8", "replace")
        d = json.loads(raw) if raw.strip() else {}
    except Exception:
        return
    try:
        prompt = str(d.get("prompt") or d.get("tool_input", {}).get("prompt") or "")
        if not prompt.strip():
            return
        low = prompt.lower()
        if len(prompt) >= 500:
            return
        if not any(v in low for v in IMPERATIVES):
            return
        if has_locator(low):
            return
        out = {"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": REMINDER}}
        sys.stdout.write(json.dumps(out))
    except Exception:
        return


if __name__ == "__main__":
    main()
