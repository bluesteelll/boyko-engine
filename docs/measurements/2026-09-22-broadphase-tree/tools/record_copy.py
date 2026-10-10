#!/usr/bin/env python
"""Copy the window-4 scratch directory into the repository's measurement directory.

Selection (the recipe's 2.4 and the record task's rule "keep the per-process JSON lines and the
criterion estimates, drop nothing that a number in analysis.md cites"):

  kept    analysis.md, window_report.md, rows.json, wait_log.txt, bin/{SHA256SUMS,COMMIT.txt},
          gate/ (the pose gate's stdout/stderr/rc and its four pose files), logs/ (minus pid files
          and the .bak), analyst/ (the analyst's reduction.json + tables.txt), tools/ (minus
          __pycache__), raw/ top-level files, raw/receipts/, raw/launch1_aborted_0819/,
          raw/pass-0{0,1}/<slot>/{stdout.txt,stderr.txt,run.csv,pose.bin} (every process; the
          pose.bin files are four distinct blobs, so git stores four objects),
          raw/g4/{G4_DONE,g4_log.txt,g4_reduction.json,runs.jsonl,wait_log.txt}, raw/g4/logs/
          (the stdout + stderr receipts of every process), raw/g4/stop1_0334/, and
          raw/g4/<group>/<arm>/<param>/k*/estimates.json.
  dropped bin/runner_c3.exe (hash in SHA256SUMS), test/ and rehearsal/ (untimed), criterion's
          benchmark.json / sample.json / tukey.json, and every criterion `new/` directory (verified
          below to be a byte-identical copy of the last-run k baseline of the same cell).
"""
import filecmp
import os
import shutil
import sys

SRC = "C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4"
DST = "D:/wt/joltab/docs/measurements/2026-09-22-broadphase-tree"

copied = []
dropped = []


def cp(rel):
    s = os.path.join(SRC, rel)
    d = os.path.join(DST, rel)
    os.makedirs(os.path.dirname(d), exist_ok=True)
    shutil.copy2(s, d)
    copied.append(rel)


def walk(rel_dir):
    base = os.path.join(SRC, rel_dir)
    for root, dirs, files in os.walk(base):
        dirs[:] = sorted(dirs)
        for f in sorted(files):
            yield os.path.relpath(os.path.join(root, f), SRC).replace("\\", "/")


# 1. top-level files
for f in ["analysis.md", "window_report.md", "rows.json", "wait_log.txt"]:
    cp(f)

# 2. bin/ (no exe)
for f in ["bin/SHA256SUMS", "bin/COMMIT.txt"]:
    cp(f)
dropped.append("bin/runner_c3.exe")

# 3. gate/, analyst/, tools/ (minus caches), logs/ (minus pid files and the .bak)
for rel in walk("gate"):
    cp(rel)
for rel in walk("analyst"):
    cp(rel)
for rel in walk("tools"):
    if "__pycache__" in rel or rel.endswith(".pyc"):
        dropped.append(rel)
        continue
    cp(rel)
for rel in walk("logs"):
    b = os.path.basename(rel)
    if b.endswith(".pid") or b.endswith(".pid.txt") or b.endswith(".bak"):
        dropped.append(rel)
        continue
    cp(rel)

# 4. raw/ top level + receipts + aborted launch + the two passes
for rel in walk("raw"):
    parts = rel.split("/")
    if parts[1] == "g4":
        continue
    cp(rel)

# 5. raw/g4: everything except the criterion payload, then estimates.json of the k* baselines
new_checked = 0
for rel in walk("raw/g4"):
    parts = rel.split("/")
    # raw/g4/<file>, raw/g4/logs/*, raw/g4/stop1_0334/*
    if len(parts) == 3 or parts[2] in ("logs", "stop1_0334"):
        cp(rel)
        continue
    # raw/g4/<group>/<arm>/<param>/<baseline>/<file>
    if len(parts) != 7:
        raise SystemExit("unexpected criterion path: " + rel)
    group, arm, param, baseline, fname = parts[2:]
    if baseline == "new":
        if fname == "estimates.json":
            cell = os.path.join(SRC, "raw/g4", group, arm, param)
            twins = [b for b in sorted(os.listdir(cell)) if b != "new"]
            same = [b for b in twins if filecmp.cmp(os.path.join(cell, "new", fname), os.path.join(cell, b, fname), shallow=False)]
            if not same:
                raise SystemExit("criterion new/ is not a copy of any k baseline: " + rel)
            new_checked += 1
        dropped.append(rel)
        continue
    if fname == "estimates.json":
        cp(rel)
    else:
        dropped.append(rel)

# 6. not copied at all
for d in ["test", "rehearsal"]:
    dropped.append(d + "/ (untimed rehearsals)")

total = sum(os.path.getsize(os.path.join(DST, r)) for r in copied)
print("copied %d files, %.1f MB" % (len(copied), total / 2**20))
print("dropped %d entries; criterion new/ directories verified as k-copies: %d" % (len(dropped), new_checked))
with open(os.path.join(SRC, "record_copy_manifest.txt"), "w", encoding="utf-8") as fh:
    fh.write("# copied\n" + "\n".join(copied) + "\n# dropped\n" + "\n".join(dropped) + "\n")
