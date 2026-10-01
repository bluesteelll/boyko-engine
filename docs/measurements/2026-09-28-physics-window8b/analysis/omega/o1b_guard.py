"""o1b: sensitivity of the park-price and route verdicts to window 8's K >= 3 guard (lib8.cmp_ refuses a claim when
either cell has K < 3). Pass 0 of omega2-gap-worker / -external lost one slot each (K = 2). Here the guard is waived
(K >= 2), everything else identical; the per-pass K=2 cells printed in full."""
import json
import math
import sys

sys.dont_write_bytecode = True
import lib_om as L

PER = json.load(open(L.HERE + '/o1_per_process.json'))
for x in PER:
    x['key'] = tuple(x['key'])
O = L.Out()


def cmp2(a, b):
    c = L.cmp_(a, b)
    if c is None:
        return None
    e = abs(c['ratio'] - 1)
    for k in ('r', 'i', 's'):
        c['cl_' + k] = a['K'] >= 2 and b['K'] >= 2 and e > c['bar_' + k]
    return c


def get(k, passes=None):
    return L.cell([x['slope'] for x in PER if x['key'] == k and (passes is None or x['pass'] in passes)])


O('# o1b: K >= 3 guard waived (K >= 2); park vs spin slope, gap rows (P8, bpp 4)')
for route in ('worker', 'external'):
    for gap in (0, 20, 80):
        ks, kp = (route, 8, 4, 'spin', gap), (route, 8, 4, 'park', gap)
        pooled = cmp2(get(ks), get(kp))
        per = {q: cmp2(get(ks, {q}), get(kp, {q})) for q in (0, 1, 2)}
        parts = [('pooled', pooled)] + [('p%d' % q, per[q]) for q in (0, 1, 2)]
        ok_is = all(c['cl_i'] and c['cl_s'] for _, c in parts)
        ok_r = all(c['cl_r'] for _, c in parts)
        O('%s gap %d: %s -> %s' % (route, gap, ' '.join('%s %s' % (n, L.yn(c)) for n, c in parts),
                                  'CLAIMED STRONG' if ok_is and ok_r else ('CLAIMED' if ok_is else 'NOT CLAIMED')))
        for q in (0,):
            a, b = get(ks, {q}), get(kp, {q})
            O('   p0 spin values %s (K %d) | park values %s (K %d) | p0 %s' % (
                [round(v) for v in a['values']], a['K'], [round(v) for v in b['values']], b['K'], L.fcmp(per[0], 0)))
O.save('o1b.txt')
