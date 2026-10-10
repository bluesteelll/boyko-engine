"""R1-R3 (runner legs) recomputed independently under ruling 1, three readings of 'every clean pass'."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

P = [p for p in json.load(open(os.path.join(L.HERE, 'v_procs.json')))['procs'] if p.get('used')]


def vals(row, W, f, passes=None):
    return [f(p) for p in P if p['row'] == row and p['W'] == W and (passes is None or p['pass'] in passes)]


def C(row, W, f, passes=None):
    return L.cell(vals(row, W, f, passes))


def fcell(c, u=''):
    return (f"{c['m']:.2f}{u} [{c['min']:.2f}-{c['max']:.2f}] K={c['K']} r {100*c['r']:.2f} i {100*c['i']:.2f} "
            f"s {100*c['s']:.2f} %")


def fcmp(c):
    return (f"ratio {c['ratio']:.4f} ({100*(c['ratio']-1):+.2f} %) {L.yn(c)} bars r {100*c['bars']['r']:.2f} "
            f"i {100*c['bars']['i']:.2f} s {100*c['bars']['s']:.2f} % K {c['KA']}/{c.get('KB','-')}")


def pair_eval(A, B, W, f, direction, label):
    """direction -1: B < A is the claim; +1: B > A (B slower)."""
    out = {}
    pooled = L.cmp(C(A, W, f), C(B, W, f))
    right_dir = (pooled['ratio'] - 1) * direction > 0
    per = []
    for ps in (0, 1, 2):
        a, b = C(A, W, f, [ps]), C(B, W, f, [ps])
        per.append((ps, a, b, L.cmp(a, b, 3), L.cmp(a, b, 2)))
    print(f'{label}: A={A} B={B} W{W}')
    print(f"   A pooled {fcell(C(A, W, f))}")
    print(f"   B pooled {fcell(C(B, W, f))}")
    print(f"   pooled  {fcmp(pooled)}  d {pooled['d']:+.3f}")
    for ps, a, b, c3, c2 in per:
        dirok = (c3['ratio'] - 1) * direction > 0
        print(f"   pass {ps}: A K{a['K']} {a['m']:.3f} B K{b['K']} {b['m']:.3f} {fcmp(c3)} | kmin2 {L.yn(c2)} dir {'ok' if dirok else 'OPPOSITE'}")
    def is_(c):
        return c['fl']['i'] and c['fl']['s'] and (c['ratio'] - 1) * direction > 0
    pooled_is = is_(pooled)
    letter = pooled_is and all(is_(c3) for _, _, _, c3, _ in per)
    k2 = pooled_is and all(is_(c2) for _, _, _, _, c2 in per)
    full = [c3 for _, a, b, c3, _ in per if a['K'] == 3 and b['K'] == 3]
    cleanr = pooled_is and all(is_(c) for c in full)
    strong_pool = pooled['fl']['r']
    print(f"   => pooled i&s {pooled_is}  LETTER {letter}  K2 {k2}  CLEAN(full passes only, {len(full)} gate) {cleanr}  pooled r {strong_pool}")
    return {'pooled': pooled, 'letter': letter, 'k2': k2, 'clean': cleanr}


tq = lambda p: p['t_q_us']
tb = lambda p: p['t_b_us']
tqb = lambda p: p['t_qb_us']
four = lambda p: p['four_us']
wall = lambda p: p['wall_ms']['0..500']

print('=== R1: F3-TD-armed-kd W1 t_q <= 186 us (constant bar; claim needs |m/b-1| > 2i and > 2s, r = STRONG)')
for ps in (None, [0], [1], [2]):
    c = C('F3-TD-armed-kd', 1, tq, ps)
    k = L.const(c, 186.0)
    print(f"   {'pooled' if ps is None else 'pass '+str(ps[0])}: {fcell(c,' us')} m/bar {k['ratio']:.4f} "
          f"({100*(k['ratio']-1):+.2f} %) {L.yn(k)} bars r {100*k['bars']['r']:.2f} i {100*k['bars']['i']:.2f} s {100*k['bars']['s']:.2f}")
    if ps is None:
        print(f"   c_q = t_q / 1240 = {1000*c['m']/1240:.2f} ns")

print()
print('=== R2: t_q kd / leaflist < 1 at W1 (TD)')
r2 = pair_eval('F3-TD-armed-leaflist', 'F3-TD-armed-kd', 1, tq, -1, 'R2 t_q')
print()
print('=== R3a: t_qb kd < leaflist at W1 (TD)')
r3a = pair_eval('F3-TD-armed-leaflist', 'F3-TD-armed-kd', 1, tqb, -1, 'R3a t_qb')
print()
print('=== R3b: t_qb kd not claimed slower at W8 (TD)')
r3b = pair_eval('F3-TD-armed-leaflist', 'F3-TD-armed-kd', 8, tqb, +1, 'R3b t_qb (kd slower?)')
print()
print('=== R3c: F3-JT / F3-RT walls [0,500) kd not claimed slower at any W')
for fam in ('JT', 'RT'):
    for W in (1, 2, 4, 8, 16):
        pair_eval(f'F3-{fam}-leaflist', f'F3-{fam}-kd', W, wall, +1, f'R3c {fam} wall (kd slower?)')
print()
print('=== context: t_b and four-span, W1 and W8 (TD)')
for W in (1, 8):
    for nm, f in (('t_q', tq), ('t_b', tb), ('t_qb', tqb), ('four', four)):
        a, b = C('F3-TD-armed-leaflist', W, f), C('F3-TD-armed-kd', W, f)
        c = L.cmp(a, b)
        print(f"   W{W} {nm}: leaflist {a['m']:.2f} (K{a['K']}) kd {b['m']:.2f} (K{b['K']}) d {c['d']:+.2f} us {fcmp(c)}")
print()
print('=== context: TA (cfg a) W1 and TR W1')
for nm, f in (('t_q', tq), ('t_b', tb), ('t_qb', tqb), ('four', four)):
    a, b = C('F3-TA-armed-leaflist', 1, f), C('F3-TA-armed-kd', 1, f)
    c = L.cmp(a, b)
    print(f"   TA W1 {nm}: leaflist {fcell(a)} | kd {fcell(b)} | {fcmp(c)}")
    t = C('F3-TR-armed', 1, f)
    print(f"   TR W1 {nm}: {fcell(t)}")
