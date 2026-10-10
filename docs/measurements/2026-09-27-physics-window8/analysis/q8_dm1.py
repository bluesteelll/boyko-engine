from common8 import *
import math

OUT.clear()
ZN = {2: 'VB_SHADE', 14: 'VB_PRODUCE_NET', 9: 'VB_RUN', 12: 'VB_PRODUCE_RUN', 4: 'VB_EARLY_CULL', 5: 'VB_EARLY_RASTER', 17: 'GBUF_DEFERRED_RESOLVE'}
dm = sorted([p for p in PROCS if p['kind'] == 'dm1'], key=lambda p: p['seq'])
P('# Q8 DM1 C6: A = cad5439b (host-visible table), B = 97ee830f (device-local); 1920x1080 only; zone median_ns over 220 frames')
P('present modes: ' + str(sorted(set(p.get('present_mode') for p in dm))))
for row in ('dm-vb-1920x1080-idle', 'dm-deferred-1920x1080-idle', 'dm-vb-1920x1080-edit100', 'dm-deferred-1920x1080-edit100'):
    ps = [p for p in dm if p['row'] == row]
    order = [p['binary'] for p in ps]
    P(f'\n## {row}: order {order} (seq {[p["seq"] for p in ps]})')
    A = [p for p in ps if p['binary'] == 'dmA']
    B = [p for p in ps if p['binary'] == 'dmB']
    zones = sorted(set(ps[0]['zones']) & set(ZN)) if 'vb' in row else [17]
    for z in sorted(set(ps[0]['zones'])):
        zi = int(z)
        if zi not in ZN:
            continue
        a1, a2 = A[0]['zones'][z]['median_ns'], A[1]['zones'][z]['median_ns']
        b1, b2 = B[0]['zones'][z]['median_ns'], B[1]['zones'][z]['median_ns']
        am = [A[0]['zones'][z]['mean_ns'], A[1]['zones'][z]['mean_ns']]
        bm = [B[0]['zones'][z]['mean_ns'], B[1]['zones'][z]['mean_ns']]
        d = statistics.median([b1 - a1, b2 - a2])
        band = max(abs(a1 - a2), abs(b1 - b2))
        verdict = 'B<=A (d<=0)' if d <= 0 else ('B<=A (inside band)' if d <= band else 'RED: B-A above band')
        P(f'  zone {zi} {ZN[zi]:22s} A1 {a1 / 1e3:8.2f} B1 {b1 / 1e3:8.2f} B2 {b2 / 1e3:8.2f} A2 {a2 / 1e3:8.2f} us; median(B-A) {d / 1e3:+7.2f} us ({100 * d / statistics.mean([a1, a2]):+.2f} %); band {band / 1e3:6.2f} us -> {verdict}; means A {statistics.mean(am) / 1e3:.2f} B {statistics.mean(bm) / 1e3:.2f}')
P('\n## the grow frame (B only, grow at frame 150): per-zone mean over 220 frames vs B idle; implied one-frame excess = 220 x (mean_grow - mean_idle) (estimate; the artifact has no per-frame record)')
Bidle = [p for p in dm if p['row'] == 'dm-vb-1920x1080-idle' and p['binary'] == 'dmB']
G = [p for p in dm if p['row'] == 'dm-vb-1920x1080-grow150']
for z in ('9', '2', '14'):
    mi = [p['zones'][z]['mean_ns'] for p in Bidle]
    mg = [p['zones'][z]['mean_ns'] for p in G]
    sd = [p['zones'][z]['stddev_ns'] for p in Bidle + G]
    p95 = ([p['zones'][z]['p95_ns'] for p in Bidle], [p['zones'][z]['p95_ns'] for p in G])
    exc = 220 * (statistics.mean(mg) - statistics.mean(mi))
    se = statistics.mean(sd) / math.sqrt(220) * 220 if False else None
    P(f'  zone {z} {ZN[int(z)]}: idle means {[round(x / 1e3, 2) for x in mi]} us; grow means {[round(x / 1e3, 2) for x in mg]} us; implied excess {exc / 1e3:+.1f} us; per-frame SD {statistics.mean(sd) / 1e3:.1f} us -> SE of a 220-frame mean {statistics.mean(sd) / math.sqrt(220) / 1e3:.2f} us, x220 = {statistics.mean(sd) * math.sqrt(220) / 1e3:.0f} us; p95 idle {p95[0]} grow {p95[1]}')
P('\n## the 100-row edit vs idle, per binary (median_ns): the edit adds')
for path, zs in (('vb', ('2', '14', '9')), ('deferred', ('17',))):
    for b in ('dmA', 'dmB'):
        for z in zs:
            i = [p['zones'][z]['median_ns'] for p in dm if p['row'] == f'dm-{path}-1920x1080-idle' and p['binary'] == b]
            e = [p['zones'][z]['median_ns'] for p in dm if p['row'] == f'dm-{path}-1920x1080-edit100' and p['binary'] == b]
            P(f'  {path} {b} zone {z}: idle {[round(x / 1e3, 2) for x in i]} edit100 {[round(x / 1e3, 2) for x in e]} us; d {(statistics.mean(e) - statistics.mean(i)) / 1e3:+.2f} us')
save('q8.txt')
