# Open questions / difficulties surfaced to the owner

Standing channel for anything ambiguous, contentious, or broken-by-tooling
that a session hits mid-task. Each entry names the date, the finding, and the
decision it needs. Resolved entries move to the bottom with the resolution.

## 2026-08-20 — clippy config leaks across git worktrees

**Finding.** Gates run inside `.claude/worktrees/<name>` inherit the MAIN
checkout's `clippy.toml` through clippy's ancestor-directory config search:
the worktree physically lives under `D:\claude\BoykoEngine`, so a worktree on
a master-based branch (no `clippy.toml`, no hot-path-ban `#[allow]`s) gets
linted against `feat/multi-paradigm-render`'s mechanical hot-path ban.
Observed as 73 `disallowed_types` errors across `boyko-macros`,
`boyko-threadpool`, `boyko_shaderdsl` — none of them real for the branch
under test, and the aborted lib lints also mask the crates that depend on
them (a false-red that can hide a true-red). `CLIPPY_CONF_DIR` does NOT fix
it (the ancestor walk continues above it); a local empty `clippy.toml` shim
does (nearest config wins).

**Decision needed.** Pick one:
1. Merge the hot-path ban (clippy.toml + the accompanying `#[allow]`
   rationale sites) to master so every branch carries the same lint config —
   the config then always matches the code being linted; or
2. Encode the shim into the gate procedure for worktrees (write an empty
   `clippy.toml` before linting, delete after), accepting that worktree
   clippy never sees the ban.

Option 1 removes the hazard class; option 2 just fences this instance.

## 2026-08-20 — two distinct dispatcher-owned event lanes (observation)

`EventDispatcher::send_event` routes the dispatcher thread to lane
`default_thread_count - 1`, while `EventWriter::send` routes it to
`buffer.thread_count - 1`. When an event type is preregistered WIDER than the
required minimum, those are two different lanes — both owned solely by the
dispatcher thread, so the EVT1 single-writer invariant holds and no race
exists, but "the dispatcher's reserved lane" is not one lane in that case.
Harmless today; worth unifying if lane accounting ever becomes observable
(e.g. per-lane diagnostics keyed by role).
