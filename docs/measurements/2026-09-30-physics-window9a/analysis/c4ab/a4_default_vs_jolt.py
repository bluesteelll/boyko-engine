"""Post hoc: what C4 changes in the standing of the SHIPPED DEFAULT against Jolt 5.6, same block. Before C4 the shipped
default is AllPairs (C4-JDpar#parent, literal; C4-JDap#tip, same config in the tip binary); after C4 it is the Tree
(C4-JD#tip). Ruling 1 + ruling 8, both readings; wall [0,500) and [100,500)."""
import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, _ = L.select('C4-AB', recs)
V = {}
for p in used:
    for wn, v in p['v'].items():
        V.setdefault((p['row'], p['binary'], p['W'], wn), {}).setdefault(p['pass'], []).append(v)
for wn in ('0..500', '100..500'):
    O(f'\n## shipped default / Jolt 5.6 [{wn}]')
    for W in (1, 2, 4, 8, 16):
        va = V[('C4-jolt56', 'j56', W, wn)]
        line = f'  W{W:2d}:'
        for row, b in (('C4-JDpar', 'parent'), ('C4-JDap', 'tip'), ('C4-JD', 'tip')):
            jl = L.judge(va, V[(row, b, W, wn)], reading='LETTER')
            jg = L.judge(va, V[(row, b, W, wn)], reading='GATING-ONLY')
            wl = ('Jolt faster' if jl['dir'] == 'B>A' else 'ours faster') if jl['claimed'] else 'n.c.'
            wg = ('Jolt faster' if jg['dir'] == 'B>A' else 'ours faster') if jg['claimed'] else 'n.c.'
            line += (f" {row}#{b} {jl['pooled']['ratio']:.4f} {L.yn(jl['pooled'])} "
                     f"[LETTER {L.label(jl)} {wl}; GATING-ONLY {L.label(jg)} {wg}] |")
        O(line)
O.save('a4_default_vs_jolt.txt')
