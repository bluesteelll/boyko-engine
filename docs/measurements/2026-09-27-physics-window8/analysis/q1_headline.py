"""Q1: ours vs Jolt 5.6 at W 1/8/16, wall and per manifold."""
from common8 import *

JOLT_M = 8489.0  # window 7 gate/jolt_receipt (not re-measured in window 8; the count does not depend on W)
P('# Q1 headline, ours (J-T, 226bd99e, reuse ON, tree, sleeping off) vs H-jolt56 (918fd2b7)')
for win in ('0..500', '100..500', '0..100'):
    P(f'\n## window [{win})')
    for W in (1, 8, 16):
        a = C('J-T', W, wall(win), ['W8S-A'])
        r = C('R-J-T', W, wall(win), ['W8S-R'])
        pool = L.cell([wall(win)(p) for p in sel('J-T', W, ['W8S-A']) + sel('R-J-T', W, ['W8S-R'])])
        j = C('H-jolt56', W, wall(win), ['W8S-A'])
        P(f'W{W}: J-T(A) {L.fc(a)}')
        P(f'     R-J-T(R) {L.fc(r)}')
        P(f'     J-T pooled A+R {L.fc(pool)}')
        P(f'     H-jolt56(A) {L.fc(j)}')
        P(f'     ours/Jolt block A: {L.fcmp(L.cmp_(j, a))}')
        P(f'     ours(R)/Jolt(A) cross-block: {L.fcmp(L.cmp_(j, r))}')
        P(f'     ours pooled/Jolt(A): {L.fcmp(L.cmp_(j, pool))}')
        P(f'     block term R-J-T/J-T: {L.fcmp(L.cmp_(a, r))}')
        for ps in ((0,), (1,)):
            a2 = C('J-T', W, wall(win), ['W8S-A'], passes=ps)
            j2 = C('H-jolt56', W, wall(win), ['W8S-A'], passes=ps)
            P(f'     pass {ps[0]} only (K=3, diag): ours {a2["median"]:.4f} Jolt {j2["median"]:.4f} ratio {a2["median"] / j2["median"]:.4f}')

P('\n## per manifold [100,500), each side own count')
for W in (1, 8, 16):
    ps = sel('J-T', W, ['W8S-A'])
    man = [p['stats']['100..500']['manifolds'] for p in ps]
    ours_pm = L.cell([p['wall_ms']['100..500'] / p['stats']['100..500']['manifolds'] * 1e6 for p in ps])
    rps = sel('R-J-T', W, ['W8S-R'])
    ours_pm_r = L.cell([p['wall_ms']['100..500'] / p['stats']['100..500']['manifolds'] * 1e6 for p in rps])
    j = C('H-jolt56', W, lambda p: p['wall_ms']['100..500'] / JOLT_M * 1e6, ['W8S-A'])
    P(f'W{W}: our manifolds/step {min(man):.4f}-{max(man):.4f}; ours ns/manifold A {L.fc(ours_pm, 1)}; R {L.fc(ours_pm_r, 1)}; Jolt {L.fc(j, 1)}')
    P(f'     ours/Jolt per manifold (A): {L.fcmp(L.cmp_(j, ours_pm))}')
    P(f'     ours(R)/Jolt per manifold: {L.fcmp(L.cmp_(j, ours_pm_r))}')

P('\n## scaling T(1)/T(W), block A [0,500)')
for row, blk in (('J-T', 'W8S-A'), ('H-jolt56', 'W8S-A'), ('R-J-T', 'W8S-R')):
    t1 = C(row, 1, wall(), [blk])['median']
    s = []
    for W in (2, 4, 8, 16):
        c = C(row, W, wall(), [blk])
        if c:
            s.append(f'W{W} {t1 / c["median"]:.3f} (E {t1 / c["median"] / W:.2f})')
    P(f'{row} ({blk}): T1 {t1:.4f}; ' + '; '.join(s))
P('\n## W16 vs W8 per row')
for row, blk in (('J-T', 'W8S-A'), ('H-jolt56', 'W8S-A'), ('R-J-T', 'W8S-R'), ('J-T-off', 'W8S-A'), ('J-A', 'W8S-A')):
    c8, c16 = C(row, 8, wall(), [blk]), C(row, 16, wall(), [blk])
    P(f'{row}: W16/W8 {L.fcmp(L.cmp_(c8, c16))}')
save('q1.txt')
