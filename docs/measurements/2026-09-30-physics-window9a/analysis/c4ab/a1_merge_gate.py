"""C4's merge gate and its controls, exactly as pre-registered (tree-c4/cut.md section 5 + addendum; lane record
07-C4-RECORD.md section 6 at 50e31f1a; jolt-gap PLAN M7 "C4 merges if no J-D W is slower and G5 is recorded"), under
ruling 1 + ruling 8, in two readings (LETTER = the verdict; GATING-ONLY = post hoc). Plus post hoc checks."""
import json

import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, dropped = L.select('C4-AB', recs)
V = {}
for p in used:
    for wn, v in p['v'].items():
        V.setdefault((p['row'], p['binary'], p['W'], wn), {}).setdefault(p['pass'], []).append(v)
RES = {}


def verdict(kind, j):
    if kind == 'not_slower':
        if j['claimed'] and j['dir'] == 'B>A':
            return 'FAILS (B claimed slower)' + (' STRONG' if j['strong'] else '')
        return 'HOLDS (B claimed faster' + (' STRONG)' if j['strong'] else ')') if j['claimed'] else 'HOLDS (not claimed)'
    if kind == 'faster':
        if j['claimed'] and j['dir'] == 'B<A':
            return 'CLAIMED' + (' STRONG' if j['strong'] else '')
        if j['claimed']:
            return 'REFUTED (B claimed slower)' + (' STRONG' if j['strong'] else '')
        return 'NOT CLAIMED'
    if kind == 'same':
        return ('CLAIMED DIFFERENT ' + j['dir'] + (' STRONG' if j['strong'] else '')) if j['claimed'] else 'NOT CLAIMED DIFFERENT'
    if kind == 'rise':
        if j['claimed'] and j['dir'] == 'B>A':
            return 'SEEN' + (' STRONG' if j['strong'] else '')
        return 'OPPOSITE' if j['claimed'] else 'NOT SEEN'


def show(key, name, ka, kb, kind, wn='0..500', d=4):
    va, vb = V.get((*ka, wn), {}), V.get((*kb, wn), {})
    O(f'\n### {name}  [{wn}] ({kind})')
    out = {}
    for rd in ('LETTER', 'GATING-ONLY'):
        j = L.judge(va, vb, reading=rd)
        out[rd] = j
    j = out['LETTER']
    O(f'  A {ka}: {L.fcell(j["a"], d)}')
    O(f'  B {kb}: {L.fcell(j["b"], d)}')
    for k in L.PASSES:
        ca, cb = L.cell(va.get(k, [])), L.cell(vb.get(k, []))
        O(f'    p{k}: A {L.fcell(ca, d)} | B {L.fcell(cb, d)}')
    for rd in ('LETTER', 'GATING-ONLY'):
        jj = out[rd]
        O(f'  {rd:11s}: {L.fj(jj, d)} => {verdict(kind, jj)}')
    RES[(key, wn)] = {rd: {'verdict': verdict(kind, out[rd]), 'label': L.label(out[rd]), 'dir': out[rd]['dir'],
                           'gates': out[rd]['gates'], 'pooled': out[rd]['pooled'],
                           'per': {str(k): v for k, v in out[rd]['per'].items()},
                           'a': out[rd]['a'], 'b': out[rd]['b']} for rd in out}
    return out


O(f'C4-AB: used {len(used)} slots, dropped {len(dropped)}')
O('\n## Pre-registered: the merge gate (C4-JD not claimed slower than C4-JDap nor C4-JDpar at any W)')
for W in (1, 2, 4, 8, 16):
    show(f'G-ap W{W}', f'G-ap W{W}: C4-JD#tip vs C4-JDap#tip (same binary, row of record)',
         ('C4-JDap', 'tip', W), ('C4-JD', 'tip', W), 'not_slower')
    show(f'G-par W{W}', f'G-par W{W}: C4-JD#tip vs C4-JDpar#parent (literal parent default)',
         ('C4-JDpar', 'parent', W), ('C4-JD', 'tip', W), 'not_slower')
O('\n## The same pairs as "claimed faster" (plan B1 arith. delta: W1 ~ -1.7, W8 -1.70, W16 ~ -1.69 ms; no bar)')
for W in (1, 2, 4, 8, 16):
    for key, A in (('F-ap', ('C4-JDap', 'tip', W)), ('F-par', ('C4-JDpar', 'parent', W))):
        for wn in ('0..500', '100..500', '0..100'):
            show(f'{key} W{W}', f'{key} W{W}: C4-JD#tip faster than {A[0]}#{A[1]}', A, ('C4-JD', 'tip', W), 'faster', wn)
O('\n## Pre-registered controls (expected NOT claimed different)')
for W in (1, 8, 16):
    show(f'C-JA W{W}', f'C-JA W{W}: C4-JA tip vs parent', ('C4-JA', 'parent', W), ('C4-JA', 'tip', W), 'same')
    show(f'C-JT W{W}', f'C-JT W{W}: C4-JT tip vs parent', ('C4-JT', 'parent', W), ('C4-JT', 'tip', W), 'same')
O('\n## Pre-registered resolution rung (PC-8b-3; canary 0.060 ms per step; reference C4-JD#tip, same W)')
for W in (8, 16):
    for wn in ('0..500', '100..500'):
        show(f'RUNG W{W}', f'RUNG W{W}: C4-rung vs C4-JD', ('C4-JD', 'tip', W), ('C4-rung', 'tip', W), 'rise', wn)
O('\n## Post hoc: A/A twin in TIP (C4-JT#tip vs C4-JD#tip, one configuration since C4)')
for W in (1, 8, 16):
    show(f'AA W{W}', f'AA W{W}: C4-JT#tip vs C4-JD#tip', ('C4-JD', 'tip', W), ('C4-JT', 'tip', W), 'same')
O('\n## Post hoc: AllPairs across binaries (C4-JDap#tip vs C4-JDpar#parent; same config)')
for W in (1, 2, 4, 8, 16):
    show(f'BIN W{W}', f'BIN W{W}: C4-JDap#tip vs C4-JDpar#parent', ('C4-JDpar', 'parent', W), ('C4-JDap', 'tip', W), 'same')
O.save('a1_merge_gate.txt')
json.dump({f'{k[0]}|{k[1]}': v for k, v in RES.items()}, open(L.os.path.join(L.HERE, 'a1_merge_gate.json'), 'w'),
          indent=1, default=str)
