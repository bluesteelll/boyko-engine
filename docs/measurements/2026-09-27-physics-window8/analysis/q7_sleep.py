from q2_identity import *

OUT.clear()
P('# Q7 sleeping ON (J-Son, cfg a, AllPairs) vs J-A (cfg a, sleeping off), block W8S-A; different pose by design')
for win in ('0..100', '100..500', '0..500'):
    for W in (1, 8):
        a = C('J-A', W, wall(win), ['W8S-A'])
        s = C('J-Son', W, wall(win), ['W8S-A'])
        aa = C('J-A-a', W, wall(win), ['W8S-A'])
        sa = C('J-Son-a', W, wall(win), ['W8S-A'])
        P(f'[{win}) W{W}: J-A {L.fc(a)}; J-Son {L.fc(s)}')
        P(f'      J-Son/J-A disarmed {L.fcmp(L.cmp_(a, s))}')
        P(f'      armed: J-A-a {aa["median"]:.4f} J-Son-a {sa["median"]:.4f}; J-Son-a/J-A-a {L.fcmp(L.cmp_(aa, sa))}')
P('\n## armed stage spans, J-A-a vs J-Son-a (ms, medians)')
for win in ('0..100', '100..500'):
    for W in (1, 8):
        ma = medterms('J-A-a', W, ['W8S-A'], win)
        ms = medterms('J-Son-a', W, ['W8S-A'], win)
        keys = ('T', 'gather', 'bp', 'np', 'graph', 'setup', 'warm', 'wide', 'narrow', 'integ_grp', 'sleep', 'restitution', 'g', 'u', 'r')
        P(f'[{win}) W{W}: ' + '; '.join(f'{k} {ma[k]:.4f}->{ms[k]:.4f}' for k in keys))
        P(f'        waves {ma["waves"]:.2f}->{ms["waves"]:.2f}; manifolds {ma["manifolds"]:.1f}->{ms["manifolds"]:.1f}')
# sleep zones individually, and the awake counts
P('\n## L10 serial pieces on J-Son-a (sleep_begin/freeze/end, classify inside bp) and awake rows')
for win in ('0..100', '100..500'):
    for W in (1, 8):
        ps = sel('J-Son-a', W, ['W8S-A'])
        for k in ('phys_sleep_begin_ns', 'phys_sleep_freeze_ns', 'phys_sleep_end_ns', 'phys_sleep_classify_ns'):
            c = L.cell([p['stats'][win].get(k, 0) / 1e3 for p in ps])
            P(f'[{win}) W{W} {k}: {c["median"]:.2f} us')
        aw = L.cell([p['stats'][win].get('awake') for p in ps])
        P(f'[{win}) W{W} awake rows {aw["median"]:.1f}; first frozen step {[p["summary"]["first_frozen_step"] for p in ps][:2]}')
ps = sel('J-Son', 8, ['W8S-A'])
P('J-Son pose hashes: ' + str(sorted(set(p['summary']['pose_hash'] for p in PROCS if p['row'].startswith('J-Son')))))
save('q7.txt')
