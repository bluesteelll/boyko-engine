#!/usr/bin/env bash
# Window 8, P-jolt56-prof: window 7's j56p recipe (tools/build_jolt_prof.sh of window 7: the same WinLibs MinGW cmake/gcc,
# Distribution, -DPROFILER_IN_DISTRIBUTION=ON, the PerformanceTest target) on a BUILD COPY of D:/tmp/jolt/wt-v5.6.0-parity
# (D:/wt/_targets/w8-jolt56prof-src, the parity patch included, .git excluded) patched by tools/patch_jolt_w8.py:
# the lighter profile + the WaitingForBatch per-thread counter. Nothing is written into the Jolt worktree.
set -u
W="C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8"
MINGW="C:/Users/flint/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin"
SRC=D:/wt/_targets/w8-jolt56prof-src
B=D:/wt/_targets/w8-jolt56prof
LOG=$W/logs/build_jolt56prof8.log
: > $LOG
echo "[$(date '+%F %T')] configure" >> $LOG
export PATH="$MINGW:$PATH"
"$MINGW/cmake.exe" -S $SRC/Build -B $B -G "MinGW Makefiles" \
  -DCMAKE_BUILD_TYPE=Distribution -DCMAKE_C_COMPILER="$MINGW/gcc.exe" -DCMAKE_CXX_COMPILER="$MINGW/c++.exe" \
  -DCMAKE_MAKE_PROGRAM="$MINGW/mingw32-make.exe" -DPROFILER_IN_DISTRIBUTION=ON >> $LOG 2>&1 || { echo "configure FAILED" >> $LOG; exit 2; }
echo "[$(date '+%F %T')] build" >> $LOG
"$MINGW/cmake.exe" --build $B --target PerformanceTest -j 12 >> $LOG 2>&1
RC=$?
echo "[$(date '+%F %T')] build exit $RC" >> $LOG
exit $RC
