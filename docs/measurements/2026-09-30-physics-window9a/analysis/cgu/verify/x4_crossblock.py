"""Verifier post hoc: C4-CGU tip cells (run 033740) against C4-AB C4-JD#tip cells (run 191354). Same exe, same args.
Own selection: closed passes of run 191354, slot = (pass, seq), original if clean+valid else first clean valid re-run."""
import io, contextlib, json, math, os, sys, collections
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
with contextlib.redirect_stdout(io.StringIO()):
    import x1_recompute as X1
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)
recs = X1.recs
ab = [r for r in recs if r.get('block') == 'C4-AB' and r.get('run_tag') == '191354']
done = {(r['pass'], r['pass_attempt']) for r in ab if r.get('pass_done')}
P('C4-AB closed passes', sorted(done))
cand = [r for r in ab if r.get('row') == 'C4-JD' and r.get('binary') == 'tip' and r.get('attempt') in ('original', 'rerun')
        and (r['pass'], r['pass_attempt']) in done]
P('C4-JD#tip records', len(cand), dict(collections.Counter(r['attempt'] for r in cand)), 'pass_attempts', dict(collections.Counter((r['pass'], r['pass_attempt']) for r in cand)))
def valid_ab(r):
    why = []
    s = r.get('summary') or {}
    if r.get('exit') != 0 or r.get('hang'): why.append('exit/hang')
    if r.get('exe_sha256') != X1.PIN['tip']: why.append('sha')
    if s.get('pose_hash') != X1.FIXHASH or s.get('expect_pose') != 'match': why.append('pose')
    if s.get('void_steps') != 0 or s.get('workers') != r['W']: why.append('void/workers')
    cfg = s.get('config') or {}
    if cfg.get('tree_brute_max_rows') != 128 or cfg.get('broadphase') != 'Tree' or cfg.get('sleeping') is not False: why.append('cfg')
    if X1.strip_paths(r['args']) != X1.row['args'] + ['--workers', str(r['W']), '--steps', '500', '--window', '0..500']: why.append('args')
    return why
slots = collections.defaultdict(list)
for r in cand:
    slots[(r['pass'], r['pass_attempt'], r['seq'])].append(r)
abused = []
dropped = 0
for k, rs in slots.items():
    orig = [x for x in rs if x['attempt'] == 'original']
    rer = sorted([x for x in rs if x['attempt'] == 'rerun'], key=lambda x: x['rerun_no'])
    pick = next((x for x in orig + rer if not valid_ab(x) and not X1.clean(x)), None)
    if pick: abused.append(pick)
    else: dropped += 1
P('AB slots', len(slots), 'used', len(abused), 'dropped', dropped)
walls = {}
def wall(r):
    if id(r) not in walls:
        walls[id(r)] = X1.csv_wall(os.path.join(r['cwd'], 'run.csv'))
    return walls[id(r)]
def mv(r, a0, b0):
    return sum(wall(r)[a0:b0]) / (b0 - a0) / 1e6
cgu_tip = [x for x in X1.used if x['binary'] == 'tip']
cgu_c1 = [x for x in X1.used if x['binary'] == 'tipcgu1']
for wn, (a0, b0) in (('[100,500)', (100, 500)), ('[0,500)', (0, 500))):
    P('')
    P('== window', wn, ': CGU/AB (post hoc) ==')
    for W in (1, 2, 4, 8, 16):
        va = {p: [mv(r, a0, b0) for r in abused if r['W'] == W and r['pass'] == p] for p in (0, 1, 2)}
        vb = {p: [mv(r, a0, b0) for r in cgu_tip if r['W'] == W and r['pass'] == p] for p in (0, 1, 2)}
        vc = [mv(r, a0, b0) for r in cgu_c1 if r['W'] == W]
        A = X1.stats(sum(va.values(), [])); B = X1.stats(sum(vb.values(), []))
        pooled = X1.comp(A, B)
        per = []
        for p in (0, 1, 2):
            if len(va[p]) >= 2 and len(vb[p]) >= 2:
                per.append(f"p{p}:{X1.yn(X1.comp(X1.stats(va[p]), X1.stats(vb[p])))}")
            else:
                per.append(f"p{p}:K{len(va[p])}")
        C = X1.stats(vc)
        P(f"W{W:<2} AB n {[len(va[p]) for p in (0,1,2)]} med {A['m']:.4f} | CGU tip med {B['m']:.4f} -> CGU/AB {100*(pooled['ratio']-1):+.2f} % pooled {X1.yn(pooled)} {' '.join(per)} | cgu1/ABtip {100*(C['m']/A['m']-1):+.2f} %")
P('')
P('== W8 / W16: per-step quantiles over [100,500) pooled over used tip processes; CPU/wall ==')
def q(xs, p):
    xs = sorted(xs); h = (len(xs) - 1) * p; lo = math.floor(h); hi = math.ceil(h)
    return xs[lo] + (xs[hi] - xs[lo]) * (h - lo)
for W in (8, 16):
    sa = [v for r in abused if r['W'] == W for v in wall(r)[100:500]]
    sb = [v for r in cgu_tip if r['W'] == W for v in wall(r)[100:500]]
    qq = ' '.join(f"p{int(100*p)} {100*(q(sb, p)/q(sa, p)-1):+.1f} %" for p in (0.1, 0.5, 0.9))
    cwa = X1.med([r['proc_cpu_s'] / r['wall_s'] for r in abused if r['W'] == W])
    cwb = X1.med([r['proc_cpu_s'] / r['wall_s'] for r in cgu_tip if r['W'] == W])
    pa = collections.Counter((r.get('summary') or {}).get('host', {}).get('power', None) for r in abused)
    P(f"W{W} CGU vs AB step quantiles: {qq} | proc CPU/wall median AB {cwa:.2f} CGU {cwb:.2f}")
P('AB used witness median', X1.med([r['others_busy_pct'] for r in abused]), 'CGU tip used witness median', X1.med([r['others_busy_pct'] for r in cgu_tip]))
open(os.path.join(HERE, 'x4_crossblock.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
