#!/usr/bin/env bash
# Window 7, P5: Jolt v5.6.0 PerformanceTest, Distribution, with the profiler compiled in (PROFILER_IN_DISTRIBUTION=ON).
# Same source tree, compiler and options as D:/tmp/jolt/build-v5.6.0-dist (P0's build); only the profiler option differs.
# Out-of-source build dir; nothing is written into the Jolt worktree.
set -u
W7=C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win7
MINGW="C:/Users/flint/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin"
B=D:/wt/_targets/win7-jolt56prof
LOG=$W7/logs/build_jolt56prof.log
: > $LOG
echo "[$(date '+%F %T')] configure" >> $LOG
export PATH="$MINGW:$PATH"
"$MINGW/cmake.exe" -S D:/tmp/jolt/wt-v5.6.0-parity/Build -B $B -G "MinGW Makefiles" \
  -DCMAKE_BUILD_TYPE=Distribution -DCMAKE_C_COMPILER="$MINGW/gcc.exe" -DCMAKE_CXX_COMPILER="$MINGW/c++.exe" \
  -DCMAKE_MAKE_PROGRAM="$MINGW/mingw32-make.exe" -DPROFILER_IN_DISTRIBUTION=ON >> $LOG 2>&1 || { echo "configure FAILED" >> $LOG; exit 2; }
echo "[$(date '+%F %T')] build" >> $LOG
"$MINGW/cmake.exe" --build $B --target PerformanceTest -j 12 >> $LOG 2>&1
RC=$?
echo "[$(date '+%F %T')] build exit $RC" >> $LOG
exit $RC
