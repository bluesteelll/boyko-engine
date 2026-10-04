"""Synthesis re-derivation of every pre-registered runner claim of window 8b (S7-AB, S4-AB, F3 runner rows, J-Son-T),
under ruling 1, in both K-guard readings (LETTER kmin 3, K2 kmin 2). Also: our trunk walls per W (for the Jolt
section, no Jolt ratio), per-manifold ns, and a cross-block comparison of the same binary + config (post hoc)."""
import json
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()
WINS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500), '274..500': (274, 500)}
V = {}  # (row, binary, W, metric) -> {pass: [values]}


def put(p, metric, v):
    V.setdefault((p['row'], p['binary'], p['W'], metric), {}).setdefault(p['pass'], []).append(v)


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


used_all = {}
for b in ('S4-AB', 'S7-AB', 'SPLIT', 'F3', 'J-Son-T'):
    _, used, dropped = L.select(b, recs)
    used_all[b] = used
    for p in used:
        c = p['c']
        for wn, (a, e) in WINS.items():
            put(p, 'wall' + wn, mean(c['wall_ns'][a:e]) / 1e6)
        for col in ('phys_solve_build_ns', 'phys_sb_pc_ns'):
            if col in c and c[col][0] is not None:
                put(p, col + '0..500', mean(c[col][0:500]) / 1e6)
        put(p, 'nsman100..500', 1e6 * mean(c['wall_ns'][100:500]) / 1e6 / mean(c['manifolds'][100:500]))
        if p['row'].startswith('F3-T') and 'armed' in p['row']:
            q = c['phys_bp_query_ns'][100:500]
            bb = c['phys_bp_build_ns'][100:500]
            put(p, 't_q', statistics.median(q) / 1e3)
            put(p, 't_b', statistics.median(bb) / 1e3)
            put(p, 't_qb', statistics.median([x + y for x, y in zip(q, bb)]) / 1e3)


def get(row, binary, W, metric):
    return V[(row, binary, W, metric)]


def show(name, ka, kb, metric, kind, bar=None, const=None, d=4):
    out = {}
    for rd, kmin in (('LETTER', 3), ('K2', 2)):
        va = get(*ka, metric)
        vb = get(*kb, metric) if kb else None
        j = L.judge(va, vb, kmin=kmin, const=const)
        out[rd] = j
    j = out['LETTER']
    a, b = j['a'], j['b']
    O(f'\n### {name}  [{metric}]')
    if const is None:
        O(f'  A {ka}: {L.fcell(a, d)}')
        O(f'  B {kb}: {L.fcell(b, d)}')
    else:
        O(f'  cell {ka}: {L.fcell(a, d)}; constant bar {const}')
    for rd in ('LETTER', 'K2'):
        jj = out[rd]
        pooled = jj['pooled']
        claimed, strong, dr = jj['claimed'], jj['strong'], jj['dir']
        if kind == 'not_slower':
            v = 'FAILS (B claimed slower)' if claimed and dr == 'B>A' else (
                'HOLDS (B claimed faster)' if claimed else 'HOLDS (no resolved difference)')
        elif kind == 'faster':
            if claimed and dr == 'B<A':
                ok = bar is None or (-pooled['delta'] >= bar)
                v = ('CLAIMED' if ok else 'CLAIMED but below bar') + (' STRONG' if strong else '')
            elif claimed:
                v = 'REFUTED (B claimed slower)' + (' STRONG' if strong else '')
            else:
                v = 'NOT CLAIMED'
        elif kind == 'slower':
            v = ('SEEN' + (' STRONG' if strong else '')) if claimed and dr == 'B>A' else (
                'OPPOSITE' if claimed else 'NOT SEEN')
        elif kind == 'below':
            v = ('CLAIMED below' + (' STRONG' if strong else '')) if claimed and dr == 'B<A' else (
                'REFUTED' if claimed else 'NOT CLAIMED')
        O(f'  {rd:6s}: {L.fj(jj, d)} => {v}')
        out[rd]['verdict'] = v
    return out


R = {}
O('# Runner claims, synthesis re-derivation (ruling 1; LETTER = kmin 3, K2 = kmin 2); window [0,500) unless named')
O('\n## S7-AB (P = s7p = trunk 16191fda, T = s7t = a3adc827); cut.md section 5 C1/C2/C3')
R['C1-JT'] = show('C1-JT T(8) vs T(16): T(16) not claimed slower', ('S7-JT', 's7t', 8), ('S7-JT', 's7t', 16),
                  'wall0..500', 'not_slower')
R['C1-JA'] = show('C1-JA T(8) vs T(16)', ('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16), 'wall0..500', 'not_slower')
R['C2-JT'] = show('C2-JT P(8) vs T(8): T(8) not claimed slower', ('S7-JT', 's7p', 8), ('S7-JT', 's7t', 8),
                  'wall0..500', 'not_slower')
R['C2-JA'] = show('C2-JA P(8) vs T(8)', ('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8), 'wall0..500', 'not_slower')
R['C3-JT'] = show('C3-JT P(16) vs T(16): T faster by >= 0.105 ms, claimed', ('S7-JT', 's7p', 16), ('S7-JT', 's7t', 16),
                  'wall0..500', 'faster', bar=0.105)
R['S7-JA16'] = show('post hoc: J-A P(16) vs T(16)', ('S7-JA', 's7p', 16), ('S7-JA', 's7t', 16), 'wall0..500', 'faster')
R['S7-W1'] = show('optional W1 pair P(1) vs T(1)', ('S7-JT-W1', 's7p', 1), ('S7-JT-W1', 's7t', 1), 'wall0..500',
                  'not_slower')
for W, rungs, inj in ((8, ('F0.5', 'F1', 'F1.5', 'F2'), (0.030, 0.060, 0.090, 0.120)),
                      (16, ('F0.5', 'F1', 'F1.5', 'F2'), (0.0525, 0.105, 0.1575, 0.210))):
    for f, i in zip(rungs, inj):
        R[f'lad{W}-{f}'] = show(f'ladder{W} {f} (injected {i} ms) vs S7-JT#s7p@W{W}', ('S7-JT', 's7p', W),
                                (f'S7-ladder{W}-{f}', 's7p', W), 'wall0..500', 'slower')
R['JA-rung16'] = show('J-A rung W16 F1 (0.105 ms)', ('S7-JA', 's7p', 16), ('S7-JA-ladder16-F1', 's7p', 16),
                      'wall0..500', 'slower')
for W in (8, 16):
    seen = [f for f in ('F0.5', 'F1', 'F1.5', 'F2') if R[f'lad{W}-{f}']['LETTER']['verdict'].startswith('SEEN')]
    O(f'  ladder{W} rungs seen (LETTER): {seen}')

O('\n## S4-AB (A = PARENT s4off, B = TIP 16191fda)')
R['S4-W8wall'] = show('S4 W8 wall: TIP faster by >= 0.060 ms', ('S4-JT', 'parent', 8), ('S4-JT', 'tip', 8),
                      'wall0..500', 'faster', bar=0.060)
R['S4-W8span'] = show('S4 W8 span phys_solve_build: gain >= 0.060 ms', ('S4-JT-a', 'parent', 8),
                      ('S4-JT-a', 'tip', 8), 'phys_solve_build_ns0..500', 'faster', bar=0.060)
R['S4-Pc'] = show('context: P-c fill phys_sb_pc W8', ('S4-JT-a', 'parent', 8), ('S4-JT-a', 'tip', 8),
                  'phys_sb_pc_ns0..500', 'faster')
R['S4-rung'] = show('S4 rung 0.060 ms at W8 (R_8 in-block)', ('S4-JT', 'tip', 8), ('S4-rung', 'tip', 8),
                    'wall0..500', 'slower')
for N in (30000, 60000):
    R[f'S4-zone{N}'] = show(f'S4 zone canary N{N} span', ('S4-JT-a', 'tip', 8), (f'S4-zone-N{N}', 'tip', 8),
                            'phys_solve_build_ns0..500', 'slower')
for W in (1, 2, 4, 16):
    R[f'S4-W{W}'] = show(f'S4 no-slower W{W}', ('S4-JT', 'parent', W), ('S4-JT', 'tip', W), 'wall0..500', 'not_slower')
for W in (8,):
    for b in ('parent', 'tip'):
        c = L.cell([v for vs in get('S4-JT', b, W, 'nsman100..500').values() for v in vs])
        O(f'  per manifold [100,500) S4-JT#{b}@W{W}: {c["median"]:.1f} ns [{c["min"]:.1f}-{c["max"]:.1f}] n={c["K"]}')

O('\n## F3 (tip 16191fda; A = leaflist, B = leaflist-kd); armed spans = median over steps [100,500), us')
R['R1'] = show('R1 t_q(kd) TD-armed W1 <= 186 us', ('F3-TD-armed-kd', 'tip', 1), None, 't_q', 'below', const=186.0,
               d=2)
R['R2'] = show('R2 t_q kd < leaflist W1', ('F3-TD-armed-leaflist', 'tip', 1), ('F3-TD-armed-kd', 'tip', 1), 't_q',
               'faster', d=2)
R['R3a'] = show('R3a t_qb kd < leaflist W1', ('F3-TD-armed-leaflist', 'tip', 1), ('F3-TD-armed-kd', 'tip', 1),
                't_qb', 'faster', d=2)
R['R3b'] = show('R3b t_qb kd not slower W8', ('F3-TD-armed-leaflist', 'tip', 8), ('F3-TD-armed-kd', 'tip', 8),
                't_qb', 'not_slower', d=2)
for W in (1, 8):
    for m in ('t_q', 't_b'):
        R[f'F3-{m}-W{W}'] = show(f'context {m} W{W}', ('F3-TD-armed-leaflist', 'tip', W), ('F3-TD-armed-kd', 'tip', W),
                                 m, 'faster', d=2)
r3c = []
for sc in ('JT', 'RT'):
    for W in (1, 2, 4, 8, 16):
        j = show(f'R3c {sc} W{W} wall kd not slower', (f'F3-{sc}-leaflist', 'tip', W), (f'F3-{sc}-kd', 'tip', W),
                 'wall0..500', 'not_slower')
        r3c.append(j['LETTER']['verdict'])
        R[f'R3c-{sc}{W}'] = j
O(f'  R3c verdicts (LETTER): {r3c}')

O('\n## J-Son-T (tip 16191fda, cfg a, tree; A = sleeping OFF, B = ON)')
for W in (1, 8):
    for arm in ('', '-a'):
        R[f'AW{W}{arm}'] = show(f'JST-AW-W{W}{arm} [0,100) ON not slower', (f'JSoffT{arm}', 'tip', W),
                                (f'JSonT{arm}', 'tip', W), 'wall0..100', 'not_slower')
        R[f'G{W}{arm}'] = show(f'JST-G-W{W}{arm} [100,500) ON faster', (f'JSoffT{arm}', 'tip', W),
                               (f'JSonT{arm}', 'tip', W), 'wall100..500', 'faster')
        R[f'F{W}{arm}'] = show(f'JST-F-W{W}{arm} [274,500) ON faster', (f'JSoffT{arm}', 'tip', W),
                               (f'JSonT{arm}', 'tip', W), 'wall274..500', 'faster')

O('\n## Reading-dependence: comparisons whose verdict differs between LETTER and K2')
diff = [(k, v['LETTER']['verdict'], v['K2']['verdict']) for k, v in R.items()
        if v['LETTER']['verdict'] != v['K2']['verdict']]
for k, a, b in diff:
    O(f'  {k}: LETTER {a} | K2 {b}')
O(f'  {len(diff)} of {len(R)} comparisons differ')

O('\n## Our trunk walls per W (J-T, tip = s7p = 16191fda, disarmed, [0,500) and [100,500); no Jolt in this window)')
for W in (1, 2, 4, 8, 16):
    c = L.cell([v for vs in get('S4-JT', 'tip', W, 'wall0..500').values() for v in vs])
    c2 = L.cell([v for vs in get('S4-JT', 'tip', W, 'wall100..500').values() for v in vs])
    m = L.cell([v for vs in get('S4-JT', 'tip', W, 'nsman100..500').values() for v in vs])
    O(f'  S4-AB TIP W{W:2d}: [0,500) {L.fcell(c)}; [100,500) {c2["median"]:.4f}; {m["median"]:.1f} ns/manifold')
for row, W in (('S7-JT-W1', 1), ('S7-JT', 8), ('S7-JT', 16)):
    c = L.cell([v for vs in get(row, 's7p', W, 'wall0..500').values() for v in vs])
    O(f'  S7-AB s7p  {row}@W{W:2d}: [0,500) {L.fcell(c)}')

O('\n## Post hoc: the same binary and the same effective config across blocks ([0,500) cell medians, ms; no claim)')
for W in (1, 2, 4, 8, 16):
    a = L.cell([v for vs in get('S4-JT', 'tip', W, 'wall0..500').values() for v in vs])
    b = L.cell([v for vs in get('F3-JT-leaflist', 'tip', W, 'wall0..500').values() for v in vs])
    O(f'  W{W:2d}: S4-AB S4-JT#tip {a["median"]:.4f} (n={a["K"]}) vs F3 F3-JT-leaflist#tip {b["median"]:.4f} '
      f'(n={b["K"]}): F3/S4 {b["median"] / a["median"]:.4f}; F3 min {b["min"]:.4f} vs S4 max {a["max"]:.4f}')
for W in (1, 8, 16):
    try:
        a = L.cell([v for vs in get('S4-JT', 'tip', W, 'wall0..500').values() for v in vs])
        row = 'S7-JT-W1' if W == 1 else 'S7-JT'
        b = L.cell([v for vs in get(row, 's7p', W, 'wall0..500').values() for v in vs])
        O(f'  W{W:2d}: S4-AB S4-JT#tip {a["median"]:.4f} vs S7-AB {row}#s7p {b["median"]:.4f}: '
          f'{b["median"] / a["median"]:.4f}')
    except KeyError:
        pass
for W in (1, 8, 16):
    a = L.cell([v for vs in get('S4-JT', 'parent', W, 'wall0..500').values() for v in vs])
    b = L.cell([v for vs in get('SPLIT-J-T', 'parent', W, 'wall0..500').values() for v in vs])
    O(f'  W{W:2d}: S4-AB S4-JT#parent {a["median"]:.4f} vs SPLIT SPLIT-J-T#parent {b["median"]:.4f}: '
      f'{b["median"] / a["median"]:.4f}')

json.dump({k: {rd: {'verdict': v[rd]['verdict'], 'A': v[rd]['pooled']['A'], 'B': v[rd]['pooled']['B'],
                    'ratio': v[rd]['pooled']['ratio'], 'delta': v[rd]['pooled']['delta'],
                    'flags': L.yn(v[rd]['pooled']), 'per': [L.yn(c) for c in v[rd]['per']], 'Ks': v[rd]['Ks'],
                    'nA': v[rd]['a']['K'], 'nB': v[rd]['b']['K'] if v[rd]['b'] else None}
               for rd in ('LETTER', 'K2')} for k, v in R.items()},
          open(L.os.path.join(L.HERE, 's1_runner_claims.json'), 'w'), indent=1)
O.save('s1_runner_claims.txt')
