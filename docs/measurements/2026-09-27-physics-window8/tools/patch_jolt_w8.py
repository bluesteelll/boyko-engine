"""Window 8, P-jolt56-prof: patch the BUILD COPY of Jolt v5.6.0 (never the parity source tree j56 was built from).

Two changes, both from the scaling design (levers/scaling/01-DESIGN.md §7 at 226bd99e; rev §9 OQ3):
  1. the lighter profile (window 7 FOLLOW-UP 10): JPH_PROFILE removed from the scopes that carry window 7's
     8,488 + 10,784 samples per frame - "Add Constraint From Cached Manifold" (8,486 calls at W1 frame 100) and the
     per-batch solve scopes ConstraintManager::sSolveVelocityConstraints + ContactConstraintManager::
     SolveVelocityConstraints (5,390 + 5,390 calls; the two together are window 7's "solve velocity 10,784 samples");
  2. Jolt's WaitingForBatch wait read by an ACCUMULATED PER-THREAD counter in JobSolveVelocityConstraints: the clock
     (rdtsc, GetProcessorTickCount) is read only at the two edges of a wait episode (the first WaitingForBatch, then
     the next batch / island / AllBatchesDone), summed in locals and emitted ONCE PER JOB into the thread's slot -
     not a JPH_PROFILE scope per yield. PerformanceTest's new -wfb flag writes wfb_<tag>.csv per frame.
Every replacement must match exactly once, or the script fails and writes nothing.
Usage: python patch_jolt_w8.py <build-copy root>
"""
import os
import sys

ROOT = sys.argv[1]
CRLF = chr(13) + chr(10)
LF = chr(10)


def edit(rel, pairs):
    p = os.path.join(ROOT, rel)
    raw = open(p, encoding='utf-8', newline='').read()
    crlf = CRLF in raw
    s = raw.replace(CRLF, LF)  # the Jolt tree is CRLF; match on LF, write back in the file's own ending
    for old, new, scope in pairs:
        if scope is not None:
            a = s.index(scope[0])
            b = s.index(scope[1], a)
            body = s[a:b]
            n = body.count(old)
            if n != 1:
                raise SystemExit(f'{rel}: {old[:60]!r} occurs {n} times inside the scope')
            s = s[:a] + body.replace(old, new) + s[b:]
        else:
            n = s.count(old)
            if n != 1:
                raise SystemExit(f'{rel}: {old[:60]!r} occurs {n} times')
            s = s.replace(old, new)
    return p, (s.replace(LF, CRLF) if crlf else s)


HEADER = r'''// boyko W8S patch (window 8, BUILD COPY ONLY - never the parity source tree): Jolt's WaitingForBatch wait, read
// by an accumulated per-thread counter emitted once per JobSolveVelocityConstraints job (scaling design
// 01-DESIGN.md section 7; rev section 9 OQ3) - not a JPH_PROFILE scope per yield, which would inflate samples.
#pragma once

#include <Jolt/Core/TickCounter.h>

JPH_SUPPRESS_WARNINGS_STD_BEGIN
#include <atomic>
JPH_SUPPRESS_WARNINGS_STD_END

JPH_NAMESPACE_BEGIN

/// One thread's accumulators, a cache line each so two threads never share one.
struct alignas(64) BoykoWaitSlot
{
	std::atomic<uint64>		mTicks { 0 };		///< processor ticks from the first WaitingForBatch of an episode to its end
	std::atomic<uint64>		mEpisodes { 0 };	///< wait episodes
	std::atomic<uint64>		mJobs { 0 };		///< JobSolveVelocityConstraints jobs that emitted
};

static constexpr int		cBoykoWaitSlots = 64;
extern BoykoWaitSlot		gBoykoWaitSlots[cBoykoWaitSlots];
extern std::atomic<uint32>	gBoykoWaitNextSlot;

/// Once per job: add the job's accumulated wait into the calling thread's slot (assigned on its first call).
void						BoykoWaitEmit(uint64 inTicks, uint64 inEpisodes);

JPH_NAMESPACE_END
'''

JOB = 'void PhysicsSystem::JobSolveVelocityConstraints('
JOB_END = 'void PhysicsSystem::JobPreIntegrateVelocity('

ps = [
    ('#include <Jolt/Core/ScopeExit.h>\n',
     '#include <Jolt/Core/ScopeExit.h>\n#include <Jolt/Physics/BoykoWaitForBatch.h> // boyko W8S patch\n', None),
    (JOB,
     '// boyko W8S patch: the WaitingForBatch counter\'s storage and its once-per-job emit\n'
     'BoykoWaitSlot gBoykoWaitSlots[cBoykoWaitSlots];\n'
     'std::atomic<uint32> gBoykoWaitNextSlot { 0 };\n\n'
     'void BoykoWaitEmit(uint64 inTicks, uint64 inEpisodes)\n'
     '{\n'
     '\tstatic thread_local int sSlot = -1;\n'
     '\tif (sSlot < 0)\n'
     '\t{\n'
     '\t\tuint32 s = gBoykoWaitNextSlot.fetch_add(1, std::memory_order_relaxed);\n'
     '\t\tsSlot = int(s < uint32(cBoykoWaitSlots)? s : uint32(cBoykoWaitSlots - 1));\n'
     '\t}\n'
     '\tBoykoWaitSlot &slot = gBoykoWaitSlots[sSlot];\n'
     '\tslot.mTicks.fetch_add(inTicks, std::memory_order_relaxed);\n'
     '\tslot.mEpisodes.fetch_add(inEpisodes, std::memory_order_relaxed);\n'
     '\tslot.mJobs.fetch_add(1, std::memory_order_relaxed);\n'
     '}\n\n' + JOB, None),
    ('\tbool check_islands = true, check_split_islands = mPhysicsSettings.mUseLargeIslandSplitter;\n',
     '\tbool check_islands = true, check_split_islands = mPhysicsSettings.mUseLargeIslandSplitter;\n'
     '\t// boyko W8S patch: the clock is read only at the two edges of a wait episode\n'
     '\tuint64 boyko_wait_ticks = 0, boyko_wait_episodes = 0, boyko_wait_start = 0;\n'
     '\tbool boyko_waiting = false;\n', (JOB, JOB_END)),
    ('\t\t\tcase LargeIslandSplitter::EStatus::BatchRetrieved:\n\t\t\t\t{\n',
     '\t\t\tcase LargeIslandSplitter::EStatus::BatchRetrieved:\n\t\t\t\t{\n'
     '\t\t\t\t\tif (boyko_waiting) { boyko_wait_ticks += GetProcessorTickCount() - boyko_wait_start; boyko_waiting = false; } // boyko W8S patch\n',
     (JOB, JOB_END)),
    ('\t\t\tcase LargeIslandSplitter::EStatus::WaitingForBatch:\n\t\t\t\tbreak;\n',
     '\t\t\tcase LargeIslandSplitter::EStatus::WaitingForBatch:\n'
     '\t\t\t\tif (!boyko_waiting) { boyko_wait_start = GetProcessorTickCount(); boyko_waiting = true; ++boyko_wait_episodes; } // boyko W8S patch\n'
     '\t\t\t\tbreak;\n', (JOB, JOB_END)),
    ('\t\t\tcase LargeIslandSplitter::EStatus::AllBatchesDone:\n\t\t\t\tcheck_split_islands = false;\n',
     '\t\t\tcase LargeIslandSplitter::EStatus::AllBatchesDone:\n'
     '\t\t\t\tif (boyko_waiting) { boyko_wait_ticks += GetProcessorTickCount() - boyko_wait_start; boyko_waiting = false; } // boyko W8S patch\n'
     '\t\t\t\tcheck_split_islands = false;\n', (JOB, JOB_END)),
    ('\t\t\tJPH_PROFILE("Island");\n',
     '\t\t\tif (boyko_waiting) { boyko_wait_ticks += GetProcessorTickCount() - boyko_wait_start; boyko_waiting = false; } // boyko W8S patch: island work ends the wait\n'
     '\t\t\tJPH_PROFILE("Island");\n', (JOB, JOB_END)),
    ('\t\telse\n\t\t{\n\t\t\t// No more work\n\t\t\tbreak;\n\t\t}\n\t}\n}\n',
     '\t\telse\n\t\t{\n\t\t\t// No more work\n\t\t\tbreak;\n\t\t}\n\t}\n\n'
     '\t// boyko W8S patch: emit once per job\n'
     '\tif (boyko_waiting)\n'
     '\t\tboyko_wait_ticks += GetProcessorTickCount() - boyko_wait_start;\n'
     '\tBoykoWaitEmit(boyko_wait_ticks, boyko_wait_episodes);\n'
     '}\n', (JOB, JOB_END)),
]

ccm = [
    ('\t\tJPH_PROFILE("Add Constraint From Cached Manifold");\n',
     '\t\t// boyko W8S patch (lighter profile): JPH_PROFILE("Add Constraint From Cached Manifold") removed\n', None),
    ('bool ContactConstraintManager::SolveVelocityConstraints(const uint32 *inConstraintOffsetBegin, const uint32 *inConstraintOffsetEnd)\n{\n\tJPH_PROFILE_FUNCTION();\n',
     'bool ContactConstraintManager::SolveVelocityConstraints(const uint32 *inConstraintOffsetBegin, const uint32 *inConstraintOffsetEnd)\n{\n\t// boyko W8S patch (lighter profile): the per-batch JPH_PROFILE_FUNCTION removed\n', None),
]

cm = [
    ('bool ConstraintManager::sSolveVelocityConstraints(Constraint **inActiveConstraints, const uint32 *inConstraintIdxBegin, const uint32 *inConstraintIdxEnd, float inDeltaTime)\n{\n\tJPH_PROFILE_FUNCTION();\n',
     'bool ConstraintManager::sSolveVelocityConstraints(Constraint **inActiveConstraints, const uint32 *inConstraintIdxBegin, const uint32 *inConstraintIdxEnd, float inDeltaTime)\n{\n\t// boyko W8S patch (lighter profile): the per-batch JPH_PROFILE_FUNCTION removed\n', None),
]

pt = [
    ('#include <atomic>\nJPH_SUPPRESS_WARNINGS_STD_END\n',
     '#include <atomic>\nJPH_SUPPRESS_WARNINGS_STD_END\n\n#include <Jolt/Physics/BoykoWaitForBatch.h> // boyko W8S patch\n', None),
    ('\tbool receipt = false; // boyko parity patch\n',
     '\tbool receipt = false; // boyko parity patch\n\tbool wfb = false; // boyko W8S patch\n', None),
    ('\t\telse if (strcmp(arg, "-receipt") == 0)\n\t\t{\n\t\t\treceipt = true;\n\t\t}\n',
     '\t\telse if (strcmp(arg, "-receipt") == 0)\n\t\t{\n\t\t\treceipt = true;\n\t\t}\n'
     '\t\telse if (strcmp(arg, "-wfb") == 0)\n\t\t{\n\t\t\twfb = true; // boyko W8S patch\n\t\t}\n', None),
    ('\tTrace("boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=%d, allow_sleep=%d, receipt=%d", no_pair_cache? 1 : 0, allow_sleep? 1 : 0, receipt? 1 : 0);\n',
     '\tTrace("boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=%d, allow_sleep=%d, receipt=%d", no_pair_cache? 1 : 0, allow_sleep? 1 : 0, receipt? 1 : 0);\n'
     '\tTrace("boyko-w8s-patch v1: lighter profile (removed: Add Constraint From Cached Manifold, ConstraintManager::sSolveVelocityConstraints, ContactConstraintManager::SolveVelocityConstraints); WaitingForBatch per-thread counter, wfb=%d", wfb? 1 : 0);\n', None),
    ('\t\t\t\tchrono::nanoseconds total_duration(0);\n',
     '\t\t\t\tchrono::nanoseconds total_duration(0);\n\n'
     '\t\t\t\t// boyko W8S patch (-wfb): per frame, each thread slot\'s WaitingForBatch ticks / episodes / jobs\n'
     '\t\t\t\tofstream wfb_file;\n'
     '\t\t\t\tif (wfb)\n'
     '\t\t\t\t{\n'
     '\t\t\t\t\twfb_file.open(("wfb_" + tag + ".csv").c_str(), ofstream::out | ofstream::trunc);\n'
     '\t\t\t\t\twfb_file << "Frame, Slot, Ticks, Episodes, Jobs" << endl;\n'
     '\t\t\t\t\tfor (BoykoWaitSlot &s : gBoykoWaitSlots) { s.mTicks.store(0); s.mEpisodes.store(0); s.mJobs.store(0); }\n'
     '\t\t\t\t}\n'
     '\t\t\t\tuint64 boyko_tsc0 = GetProcessorTickCount();\n'
     '\t\t\t\tchrono::steady_clock::time_point boyko_clk0 = chrono::steady_clock::now();\n', None),
    ('\t\t\t\t\t\treceipt_listener.Reset();\n\t\t\t\t\t}\n',
     '\t\t\t\t\t\treceipt_listener.Reset();\n\t\t\t\t\t}\n\n'
     '\t\t\t\t\t// boyko W8S patch (-wfb): outside the timed pair\n'
     '\t\t\t\t\tif (wfb)\n'
     '\t\t\t\t\t\tfor (int s = 0; s < cBoykoWaitSlots; ++s)\n'
     '\t\t\t\t\t\t{\n'
     '\t\t\t\t\t\t\tuint64 t = gBoykoWaitSlots[s].mTicks.exchange(0, memory_order_relaxed);\n'
     '\t\t\t\t\t\t\tuint64 e = gBoykoWaitSlots[s].mEpisodes.exchange(0, memory_order_relaxed);\n'
     '\t\t\t\t\t\t\tuint64 j = gBoykoWaitSlots[s].mJobs.exchange(0, memory_order_relaxed);\n'
     '\t\t\t\t\t\t\tif (t != 0 || e != 0 || j != 0)\n'
     '\t\t\t\t\t\t\t\twfb_file << iterations << ", " << s << ", " << t << ", " << e << ", " << j << endl;\n'
     '\t\t\t\t\t\t}\n', None),
    ('\t\t\t\t// Calculate hash of all positions and rotations of the bodies\n',
     '\t\t\t\t// boyko W8S patch: the tick rate of the WaitingForBatch counter, calibrated over the whole run\n'
     '\t\t\t\t{\n'
     '\t\t\t\t\tuint64 boyko_tsc1 = GetProcessorTickCount();\n'
     '\t\t\t\t\tdouble boyko_s = chrono::duration<double>(chrono::steady_clock::now() - boyko_clk0).count();\n'
     '\t\t\t\t\tTrace("boyko-w8s-patch: tsc ticks per second %.1f over %.3f s, threads slots used %u", boyko_s > 0.0? double(boyko_tsc1 - boyko_tsc0) / boyko_s : 0.0, boyko_s, gBoykoWaitNextSlot.load());\n'
     '\t\t\t\t}\n\n'
     '\t\t\t\t// Calculate hash of all positions and rotations of the bodies\n', None),
]

out = []
out.append(edit('Jolt/Physics/PhysicsSystem.cpp', ps))
out.append(edit('Jolt/Physics/Constraints/ContactConstraintManager.cpp', ccm))
out.append(edit('Jolt/Physics/Constraints/ConstraintManager.cpp', cm))
out.append(edit('PerformanceTest/PerformanceTest.cpp', pt))
hp = os.path.join(ROOT, 'Jolt', 'Physics', 'BoykoWaitForBatch.h')
if os.path.exists(hp):
    raise SystemExit('already patched')
for p, s in out:
    open(p, 'w', encoding='utf-8', newline='').write(s)
open(hp, 'w', encoding='utf-8', newline='\n').write(HEADER)
print('patched:', [os.path.relpath(p, ROOT) for p, _ in out] + ['Jolt/Physics/BoykoWaitForBatch.h'])
