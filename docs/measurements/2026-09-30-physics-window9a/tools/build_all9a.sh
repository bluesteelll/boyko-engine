#!/usr/bin/env bash
# Window 9a: the three builds, one after another (each in its own target dir; window 8b's recipe).
W9="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
H="$W9/tools"
bash "$H/build_runner.sh" D:/wt/_targets/w9a-trees/tip D:/wt/_targets/w9a-tip "$W9/logs/build_tip.log"; echo "tip rc $?" >> "$W9/logs/build_all.txt"
bash "$H/build_runner.sh" D:/wt/_targets/w9a-trees/parent D:/wt/_targets/w9a-parent "$W9/logs/build_parent.log"; echo "parent rc $?" >> "$W9/logs/build_all.txt"
bash "$H/build_g4rT.sh" D:/wt/_targets/w9a-trees/g4rT D:/wt/_targets/w9a-g4rT "$W9/logs/build_g4rT.log"; echo "g4rT rc $?" >> "$W9/logs/build_all.txt"
echo "all done" >> "$W9/logs/build_all.txt"
