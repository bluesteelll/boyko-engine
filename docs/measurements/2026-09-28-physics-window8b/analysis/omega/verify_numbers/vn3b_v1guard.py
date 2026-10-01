"""CONT-v1 under the SAME guard waiver the analyst used for PARK-20/80 (U1): every region cell, 8b (1 pass) vs window 8
pooled and vs each window-8 pass."""
import sys
sys.dont_write_bytecode = True
import vn_lib as L
import vn3_v1cont as V  # re-runs its print block; the objects below are what is used

print()
print('## guard waived: region cells 8b vs w8 (pooled, and 8b vs each w8 pass)')
for route in ('worker', 'external'):
    for P in (2, 4, 8, 16):
        for st in (36, 72):
            b = V.rcell(V.v1, route, st, P)
            a = V.rcell(V.w8used, route, st, P)
            a0 = V.rcell([p for p in V.w8used if p['pass'] == 0], route, st, P)
            a1 = V.rcell([p for p in V.w8used if p['pass'] == 1], route, st, P)
            c = L.cmp2(a, b, False)
            c0 = L.cmp2(a0, b, False)
            c1 = L.cmp2(a1, b, False)
            tag = ''
            if c['i'] and c['s']:
                tag = '  <- i AND s pooled'
                if c0['i'] and c0['s'] and c1['i'] and c1['s']:
                    tag += ', and vs each w8 pass'
            print('  %-8s P%-2d st%d: w8 %.0f (K %d) 8b %.0f (K %d) %+.1f %% pooled %s | vs w8 p0 %s p1 %s%s'
                  % (route, P, st, a['med'], a['K'], b['med'], b['K'], 100 * (c['ratio'] - 1), L.flags(c), L.flags(c0),
                     L.flags(c1), tag))
