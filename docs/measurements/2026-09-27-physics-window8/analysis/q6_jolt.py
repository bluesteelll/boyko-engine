from common8 import *
import jprof

OUT.clear()
JOBS = ['ApplyGravity', 'UpdateBroadPhasePrepare', 'FindCollisions', 'FinalizeIslands', 'SolveVelocityConstraints',
        'IntegrateVelocity', 'SolvePositionConstraints', 'DetermineActiveConstraints', 'BuildIslandsFromConstraints',
        'SetupVelocityConstraints', 'ContactRemovedCallbacks', 'UpdateBroadPhaseFinalize', 'SoftBodyPrepare',
        'ResolveCCDContacts', 'Build Jobs', 'Island']
UPD = 'JPH::EPhysicsUpdateError JPH::PhysicsSystem::Update'


def frame_stats(path):
    r = jprof.parse(path)
    ivs = r['ivs']
    upd = [iv for iv in ivs if iv[1].startswith(UPD)][0]
    out = {'update': upd[3] - upd[2]}
    for j in JOBS:
        js = [(a, b) for (t, n, a, b, d) in ivs if n == j]
        if not js:
            continue
        out[j + '.first'] = min(a for a, b in js) - upd[2]
        out[j + '.last'] = max(b for a, b in js) - upd[2]
        out[j + '.extent'] = out[j + '.last'] - out[j + '.first']
        out[j + '.union'] = jprof.union(js)
        out[j + '.cpu'] = sum(b - a for a, b in js)
        out[j + '.n'] = len(js)
    isl = [(a, b) for (t, n, a, b, d) in ivs if n == 'Island']
    big = max(isl, key=lambda x: x[1] - x[0]) if isl else (0, 0)
    out['island_big'] = big[1] - big[0]
    out['sort'] = sum(b - a for (t, n, a, b, d) in ivs if n.startswith('void JPH::ContactConstraintManager::SortContacts'))
    out['split'] = sum(b - a for (t, n, a, b, d) in ivs if n.startswith('bool JPH::LargeIslandSplitter::SplitIsland'))
    fc, sv, sp = out.get('FindCollisions.extent', 0), out.get('SolveVelocityConstraints.extent', 0), out.get('SolvePositionConstraints.extent', 0)
    out['par_solve_vel'] = sv - out['island_big']
    out['serial_rest'] = out['update'] - fc - sv - sp
    out['serial_chain'] = out['serial_rest'] + out['island_big']
    return out


res = {}
for W in (1, 8, 16):
    per = []
    for p in PROCS:
        if p['row'] != 'P-jolt56-prof' or p['W'] != W:
            continue
        fs = [frame_stats(os.path.join(p['cwd'], f'profile_chart_discrete_th{W}_it{it}.html')) for it in (100, 200, 300, 400)]
        keys = set().union(*fs)
        per.append({k: statistics.mean([f.get(k, 0.0) for f in fs]) for k in keys})
        per[-1]['frames_mean_100_500'] = p['wall_ms']['100..500']
        per[-1]['dumpframes_harness'] = statistics.mean([p['frames_ms'][it] for it in (100, 200, 300, 400)])
        # wfb
        wf = [f for f in os.listdir(p['cwd']) if f.startswith('wfb_')][0]
        rows = [tuple(int(x) for x in ln.split(',')) for ln in open(os.path.join(p['cwd'], wf)).read().splitlines()[1:]]
        tps = p['tsc']
        byf = {}
        for fr, sl, t, e, j in rows:
            byf.setdefault(fr, []).append((t / tps * 1e3, e, j))
        sel_f = [f for f in range(100, 500)]
        per[-1]['wfb_sum'] = statistics.mean([sum(x[0] for x in byf.get(f, [])) for f in sel_f])
        per[-1]['wfb_max'] = statistics.mean([max((x[0] for x in byf.get(f, [])), default=0) for f in sel_f])
        per[-1]['wfb_min'] = statistics.mean([min((x[0] for x in byf.get(f, [])), default=0) for f in sel_f])
        per[-1]['wfb_eps'] = statistics.mean([sum(x[1] for x in byf.get(f, [])) for f in sel_f])
        # the (W-1) smallest waits, i.e. excluding the thread that did the island prep (the smallest wait)
        per[-1]['wfb_others_mean'] = statistics.mean([statistics.mean(sorted(x[0] for x in byf[f])[1:]) if len(byf.get(f, [])) > 1 else 0 for f in sel_f])
    res[W] = per
    cellk = lambda k: L.cell([x.get(k, 0.0) for x in per])
    P(f'\n## Jolt j56p W{W}: K={len(per)} processes x 4 dump frames (100/200/300/400), per-process mean over the frames, cell median')
    P(f"  harness frame mean [100,500) {L.fc(cellk('frames_mean_100_500'))}; the dump frames {L.fc(cellk('dumpframes_harness'))}; Update scope {L.fc(cellk('update'))}")
    for j in JOBS:
        c = cellk(j + '.extent')
        if c and c['median'] > 0:
            P(f"  {j:28s} start {cellk(j + '.first')['median']:.4f} end {cellk(j + '.last')['median']:.4f} extent {c['median']:.4f} union {cellk(j + '.union')['median']:.4f} cpu {cellk(j + '.cpu')['median']:.4f} n {cellk(j + '.n')['median']:.0f}")
    P(f"  island_big (sort+split, one thread) {L.fc(cellk('island_big'))}; sort {cellk('sort')['median']:.4f} split {cellk('split')['median']:.4f}")
    P(f"  SolveVelocity extent - island = parallel velocity solve {L.fc(cellk('par_solve_vel'))}")
    P(f"  serial remainder (Update - FC - SV - SP extents) {L.fc(cellk('serial_rest'))}; serial chain incl. island {L.fc(cellk('serial_chain'))}")
    P(f"  WaitingForBatch [100,500): sum over threads {L.fc(cellk('wfb_sum'))} ms/frame; max thread {cellk('wfb_max')['median']:.4f}; min thread {cellk('wfb_min')['median']:.4f}; mean of the W-1 largest {cellk('wfb_others_mean')['median']:.4f}; episodes/frame {cellk('wfb_eps')['median']:.1f}")
json.dump({str(k): v for k, v in res.items()}, open(os.path.join(L.HERE, 'q6.json'), 'w'))
save('q6.txt')
