"""v1: every C4-AB comparison under ruling 1 + ruling 8 LETTER (kmin 3) and GATING-ONLY (passes p1,p2 gate, pooled all)."""
import json
import vlib as L

o = L.Out()
recs = L.records()
procs, used, dropped, slots = L.select('C4-AB', recs)


def vals(row, binary, W, key='m0_500'):
    d = {}
    for p in used:
        if p['row'] == row and p['binary'] == binary and p['W'] == W:
            d.setdefault(p['pass_'], []).append(p[key])
    return d


def both(A, B, W, key='m0_500', scale=1.0, title=''):
    va, vb = vals(*A, W, key), vals(*B, W, key)
    jl = L.judge(va, vb, kmin=3, scale=scale)
    # GATING-ONLY: pooled over all used processes, per-pass gate only on p1, p2 (both cells K>=3 there)
    jg = L.judge(va, vb, kmin=3, scale=scale, passes=(0, 1, 2))
    per_g = [c for k, c in enumerate(jg['per']) if c['KA'] >= 3 and c['KB'] >= 3]
    same = all((c['ratio'] > 1) == (jg['pooled']['ratio'] > 1) for c in per_g)
    g_cl = jg['pooled']['cl_i'] and jg['pooled']['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per_g) and same
    g_st = g_cl and jg['pooled']['cl_r'] and all(c['cl_r'] for c in per_g)
    glab = 'NOT CLAIMED' if not g_cl else ('CLAIMED STRONG' if g_st else 'CLAIMED')
    o(f"{title} W{W} [{key}] A={A[0]}#{A[1]} B={B[0]}#{B[1]}")
    o(f"   A {L.fc(jl['a'])}  Ks {[len(va.get(k, [])) for k in (0, 1, 2)]}")
    o(f"   B {L.fc(jl['b'])}  Ks {[len(vb.get(k, [])) for k in (0, 1, 2)]}")
    o(f"   LETTER {L.fj(jl)}")
    o(f"   GATING-ONLY ({len(per_g)} gating passes) -> {glab}")
    return {'letter': L.label(jl), 'dir': jl['dir'], 'gating': glab, 'ratio': jl['pooled']['ratio'],
            'delta': jl['pooled']['delta'], 'A': jl['a'], 'B': jl['b'],
            'pooled_yn': L.yn(jl['pooled']), 'per_yn': [L.yn(c) + ('(sep)' if c['sep'] else '') for c in jl['per']]}


res = {}
o('==== MG-ap: JD#tip (B) vs JDap#tip (A), [0,500)')
for W in (1, 2, 4, 8, 16):
    res[f'MG-ap W{W}'] = both(('C4-JDap', 'tip'), ('C4-JD', 'tip'), W, title='MG-ap')
o('==== MG-par: JD#tip (B) vs JDpar#parent (A), [0,500)')
for W in (1, 2, 4, 8, 16):
    res[f'MG-par W{W}'] = both(('C4-JDpar', 'parent'), ('C4-JD', 'tip'), W, title='MG-par')
o('==== MG [100,500) deltas')
for W in (1, 2, 4, 8, 16):
    a = L.cell([x for v in vals('C4-JDap', 'tip', W, 'm100_500').values() for x in v])
    b = L.cell([x for v in vals('C4-JD', 'tip', W, 'm100_500').values() for x in v])
    c = L.cell([x for v in vals('C4-JDpar', 'parent', W, 'm100_500').values() for x in v])
    o(f"   W{W}: JD-JDap {b['median'] - a['median']:+.4f}  JD-JDpar {b['median'] - c['median']:+.4f} ms")
o('==== CTL J-A: JA#tip (B) vs JA#parent (A)')
for W in (1, 8, 16):
    res[f'CTL-JA W{W}'] = both(('C4-JA', 'parent'), ('C4-JA', 'tip'), W, title='CTL-JA')
o('==== CTL J-T: JT#tip (B) vs JT#parent (A)')
for W in (1, 8, 16):
    res[f'CTL-JT W{W}'] = both(('C4-JT', 'parent'), ('C4-JT', 'tip'), W, title='CTL-JT')
o('==== AllPairs across binaries: JDap#tip (B) vs JDpar#parent (A)')
for W in (1, 2, 4, 8, 16):
    res[f'AP-x W{W}'] = both(('C4-JDpar', 'parent'), ('C4-JDap', 'tip'), W, title='AP-x')
o('==== A/A twin: JT#tip (B) vs JD#tip (A)')
for W in (1, 8, 16):
    res[f'AA W{W}'] = both(('C4-JD', 'tip'), ('C4-JT', 'tip'), W, title='AA')
o('==== RUNG: rung#tip (B) vs JD#tip (A)')
for W in (8, 16):
    res[f'RUNG W{W}'] = both(('C4-JD', 'tip'), ('C4-rung', 'tip'), W, title='RUNG')
o('==== JOLT wall: JD#tip (B) vs jolt56 (A), [0,500)')
for W in (1, 2, 4, 8, 16):
    res[f'JOLT wall W{W}'] = both(('C4-jolt56', 'j56'), ('C4-JD', 'tip'), W, title='JOLT')
o('==== JOLT wall [100,500)')
for W in (1, 2, 4, 8, 16):
    res[f'JOLT wall100 W{W}'] = both(('C4-jolt56', 'j56'), ('C4-JD', 'tip'), W, key='m100_500', title='JOLT100')
# per-unit, [100,500): ours per unit / Jolt per unit = (T_o/T_j) * (U_j/U_o)
Mo, Po = 4467.665, 16553.755  # re-derived in v2_census from C4-G5 armed rows
Mj, Pj = 8489.0, 31111.958      # DOSSIER (window 8 counting run, hash equal)
vro = Po * 3 * 12
vrj = (Pj + 3 * Mj) * 10
prj = Pj * 2
units = {'velocity row': vrj / vro, 'all rows': (vrj + prj) / vro, 'manifold': Mj / Mo}
o(f'units: ours vrow {vro:,.1f}; Jolt vrow {vrj:,.1f} + pos {prj:,.1f}; Jolt/ours: '
  + ', '.join(f'{k} {v:.4f}' for k, v in units.items()))
for name, k in units.items():
    o(f'==== JOLT per {name} [100,500) (scale {k:.4f})')
    for W in (1, 2, 4, 8, 16):
        res[f'JOLT per {name} W{W}'] = both(('C4-jolt56', 'j56'), ('C4-JD', 'tip'), W, key='m100_500', scale=k, title=f'per-{name}')
o('==== JDpar vs Jolt (shipped default before C4), [0,500)')
for W in (1, 2, 4, 8, 16):
    res[f'JDpar/Jolt W{W}'] = both(('C4-jolt56', 'j56'), ('C4-JDpar', 'parent'), W, title='JDpar-Jolt')
o('')
o('==== SUMMARY')
for k, v in res.items():
    o(f"{k:28s} ratio {v['ratio']:.4f} delta {v['delta']:+.4f} pooled {v['pooled_yn']} per {v['per_yn']} "
      f"LETTER {v['letter']} [{v['dir']}] | GATING {v['gating']}")
json.dump({k: {kk: vv for kk, vv in v.items()} for k, v in res.items()},
          open(L.os.path.join(L.HERE, 'v1_claims.json'), 'w'), indent=1, default=str)
o.save('v1_claims.txt')
