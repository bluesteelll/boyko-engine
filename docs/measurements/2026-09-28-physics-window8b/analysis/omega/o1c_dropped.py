"""o1c (post hoc sensitivity): the two dropped gap-row slots (p0 r1 omega2-gap-external, p0 r2 omega2-gap-worker)
put back, once with their original and once with their re-run, so pass 0 has K = 3; park vs spin slope at P8 bpp4
gap 20/80 re-judged with window 8's K >= 3 guard intact. Also every dirty process's slopes, to show the drop did not
remove an outlier."""
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
procs = L.all_procs()
used, dropped, slots = L.select(procs)


def slopes(p):
    d = {}
    for s in p['summaries']:
        k = (s['route'], s['participants'], s['blocks_per_participant'], s['helper'], s['gap_us'])
        d.setdefault(k, {})[s['stages']] = s['region_ns_median']
    return {k: (v[72] - v[36]) / 36.0 for k, v in d.items()}


O('# o1c: dropped gap-row slots put back (post hoc)')
for k, ps in dropped:
    if k[0] != 'omega-v2':
        continue
    for p in ps:
        sl = slopes(p)
        O('slot %s %s (%s): %s' % (k, p['attempt'], p['clean_why'], {kk[3] + '@' + str(kk[4]): round(v) for kk, v in sl.items()}))
for pick in ('original', 'rerun'):
    extra = [p for k, ps in dropped if k[0] == 'omega-v2' for p in ps if p['attempt'] == pick]
    pool = [p for p in used if p['row'].startswith('omega2-gap')] + extra
    O('\n## with the dropped slots filled by their %s' % pick)
    for route in ('worker', 'external'):
        for gap in (20, 80):
            ks, kp = (route, 8, 4, 'spin', gap), (route, 8, 4, 'park', gap)

            def c(key, q=None):
                return L.cell([slopes(p)[key] for p in pool if p['summaries'][0]['route'] == route and (q is None or p['pass'] == q)])
            pooled = L.cmp_(c(ks), c(kp))
            per = {q: L.cmp_(c(ks, q), c(kp, q)) for q in (0, 1, 2)}
            v, det = L.rule1(pooled, per)
            O('%s gap %d: spin %s | park %s | %s | %s -> %s' % (route, gap, L.fc(c(ks), 0), L.fc(c(kp), 0),
                                                              L.fcmp(pooled, 0), det, v))
O.save('o1c.txt')
