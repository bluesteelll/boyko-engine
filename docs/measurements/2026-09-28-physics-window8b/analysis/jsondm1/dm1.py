"""DM1 C6 re-read (ruling 9): A = dm1_A.exe (cad5439b, host-visible table), B = dm1_B.exe (97ee830f = C6, device-local),
ABBA x 2 per row at 1920x1080, one pass, FIFO. Gate (dm1/cut.md 5 + 11.5): per zone, the paired median of B-A against
the band max(|A1-A2|, |B1-B2|) from the ABBA's own repeats; B <= A when the median is <= 0 or inside the band; above
the band = red for C6. Per-zone statistic = the artifact's median_ns over 220 frames (the gated number, window 8's);
mean_ns and the frame span (largest end_off_ns = median per-frame end of the last zone) are post hoc.
Readings of the ABBA x 2 extension (the ruling does not spell it out):
  R1 pooled: pairs (A1,B1) (A2,B2) (A3,B3) (A4,B4) by ABBA position; d = median of the complete pairs;
     band = max(range(A repeats), range(B repeats)) over all used repeats.
  R2 per ABBA: window 8's rule verbatim on each half (A1 B1 B2 A2 | A3 B3 B4 A4).
  R3 pooled pairs, band = max of the within-half |A1-A2|, |B1-B2|, |A3-A4|, |B3-B4|.
Each reading on the slot-rule selection (clean + valid; primary) and on all originals (sensitivity)."""
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import libjd as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


ZN = {2: 'VB_SHADE', 4: 'VB_EARLY_CULL', 9: 'VB_RUN', 14: 'VB_PRODUCE_NET', 17: 'GBUF_DEFERRED_RESOLVE',
      5: 'VB_EARLY_RASTER', 12: 'VB_PRODUCE_RUN'}
GATED = {'vb': (2, 4, 9, 14), 'deferred': (17,)}
EXTRA = {'vb': (5, 12), 'deferred': ()}
recs = L.load_recs('DM1')
procs, used, dropped = L.select(recs, L.validate_dm1, dm1=True)
P('# DM1 C6 re-read, window 8b (ABBA x 2 per row, 1920x1080, one pass)')
n_o = sum(p['attempt'] == 'original' for p in procs)
n_r = sum(p['attempt'] == 'rerun' for p in procs)
P(f'processes {len(procs)} (originals {n_o}, re-runs {n_r}); slots {len(used) + len(dropped)}; used {len(used)}; dropped {len(dropped)}')
bad = [(p['seq'], p['attempt'], p['valid_why']) for p in procs if p['valid_why']]
P(f'validity problems: {len(bad)} {bad}')
P('present modes: ' + str(sorted(set(p['_pm'] for p in procs))))
frames = sorted(set((p['_line'] or '').split('frames=')[-1] for p in procs))
zn = sorted(set(int(z['n']) for p in procs for zid, z in p['_zones'].items() if zid in ZN))
P(f'frames in the DM1 line: {frames}; zone n over the read zones: {zn}')
for p in procs:
    if p['_line'] and not p['_line'].endswith('frames=241'):
        P(f"  note: seq {p['seq']} {p['attempt']} {p['row']} {p['binary']}: {p['_line']}")
for k, why in dropped:
    P(f'DROPPED slot seq {k[5]} {k[2]} {k[3]}: {why}')
for p in used:
    if p['attempt'] == 'rerun':
        P(f"re-run used: seq {p['seq']} {p['row']} {p['binary']} (started {p['start'][11:19]}, after the ABBA sequence)")

ROWS = ['dm-vb-1920x1080-idle', 'dm-deferred-1920x1080-idle', 'dm-vb-1920x1080-edit100']
POS = ['A1', 'B1', 'B2', 'A2', 'A3', 'B3', 'B4', 'A4']
VERD = {}


def val(p, z, stat='median_ns'):
    return p['_zones'][z][stat] / 1e3 if p is not None and z in p['_zones'] else None


def span(p):
    return max(zz['end_off_ns'] for zz in p['_zones'].values()) / 1e3 if p is not None else None


def gate(d, band):
    if d is None:
        return 'n/a'
    if d <= 0:
        return 'B<=A (d<=0)'
    if d <= band:
        return 'B<=A (inside band)'
    return 'RED: B-A above band'


def fnum(x, fmt):
    return 'n/a' if x is None else format(x, fmt)


def readings(slot, getv):
    v = {k: (getv(slot[k]) if slot.get(k) is not None else None) for k in POS}
    pairs = [(v['A1'], v['B1']), (v['A2'], v['B2']), (v['A3'], v['B3']), (v['A4'], v['B4'])]
    diffs = [b - a for a, b in pairs if a is not None and b is not None]
    As = [v[k] for k in ('A1', 'A2', 'A3', 'A4') if v[k] is not None]
    Bs = [v[k] for k in ('B1', 'B2', 'B3', 'B4') if v[k] is not None]
    out = {'v': v, 'n_pairs': len(diffs), 'nA': len(As), 'nB': len(Bs), 'diffs': diffs}
    d1 = statistics.median(diffs) if diffs else None
    band1 = max(max(As) - min(As), max(Bs) - min(Bs))
    out['R1'] = (d1, band1, gate(d1, band1))
    halves = []
    for a1, b1, b2, a2 in (('A1', 'B1', 'B2', 'A2'), ('A3', 'B3', 'B4', 'A4')):
        dd = [v[b] - v[a] for a, b in ((a1, b1), (a2, b2)) if v[a] is not None and v[b] is not None]
        ws = [abs(v[x] - v[y]) for x, y in ((a1, a2), (b1, b2)) if v[x] is not None and v[y] is not None]
        bd = max(ws) if ws else None
        d = statistics.median(dd) if dd else None
        halves.append((d, bd, gate(d, bd) if bd is not None else 'n/a (no repeat pair)', len(dd)))
    out['R2'] = halves
    within = [abs(v[x] - v[y]) for x, y in (('A1', 'A2'), ('B1', 'B2'), ('A3', 'A4'), ('B3', 'B4'))
              if v[x] is not None and v[y] is not None]
    band3 = max(within)
    out['R3'] = (d1, band3, gate(d1, band3))
    out['A_cell'] = L.cell(As)
    out['B_cell'] = L.cell(Bs)
    return out


def show(v):
    return ' '.join(k + ' ' + ('-' if v[k] is None else format(v[k], '.1f')) for k in POS)


def mark(p):
    if p is None:
        return '-'
    return 'R' if p['attempt'] == 'rerun' else 'o'


SELS = (('slot rule (clean + valid)', used),
        ('all originals (sensitivity)', [p for p in procs if p['attempt'] == 'original']))
for sel_name, pool in SELS:
    P('')
    P('######## selection: ' + sel_name)
    for row in ROWS:
        rp = sorted([p for p in procs if p['row'] == row and p['attempt'] == 'original'], key=lambda p: p['seq'])
        seqs = [p['seq'] for p in rp]
        order = [p['binary'][-1] for p in rp]
        slot = {}
        for pos, s, b in zip(POS, seqs, order):
            assert pos[0] == b, (row, pos, b)
            cand = [p for p in pool if p['seq'] == s and p['row'] == row]
            slot[pos] = cand[0] if cand else None
        path = 'vb' if '-vb-' in row else 'deferred'
        usedmap = ' '.join(k + '=' + mark(slot[k]) for k in POS)
        P('')
        P(f'## {row}: order {"".join(order)} seq {seqs}; used {usedmap}')
        for z in GATED[path] + EXTRA[path]:
            tag = '' if z in GATED[path] else ' (not gated; post hoc)'
            for stat in ('median_ns', 'mean_ns'):
                rd = readings(slot, lambda p: val(p, z, stat))
                d1, b1, g1 = rd['R1']
                d3, b3, g3 = rd['R3']
                avals = [x for x in (rd['v'][k] for k in ('A1', 'A2', 'A3', 'A4')) if x is not None]
                amean = statistics.mean(avals)
                h = rd['R2']
                st = 'per-frame median' if stat == 'median_ns' else 'per-frame mean (post hoc)'
                P(f'  zone {z} {ZN[z]}{tag} [{st}] us: {show(rd["v"])}')
                dl = [round(x, 2) for x in rd['diffs']]
                P(f'     R1 pooled: median(B-A) {d1:+.2f} us ({100 * d1 / amean:+.2f} % of mean A) over {rd["n_pairs"]} pairs {dl}; '
                  f'band {b1:.2f} (A range over {rd["nA"]}, B range over {rd["nB"]}) -> {g1}')
                hs = []
                for i, x in enumerate(h):
                    hs.append('ABBA' + str(i + 1) + ': d ' + fnum(x[0], '+.2f') + ' band ' + fnum(x[1], '.2f') + ' -> ' + x[2])
                P('     R2 halves: ' + ' | '.join(hs))
                P(f'     R3 pooled pairs, within-half band {b3:.2f} -> {g3}')
                if stat == 'median_ns':
                    c = L.cmp_(rd['A_cell'], rd['B_cell'])
                    P(f'     post hoc ruling-1 statistics B/A (K {rd["A_cell"]["K"]}/{rd["B_cell"]["K"]}): {L.fcmp(c)}')
                    if sel_name.startswith('slot'):
                        VERD[(row, z)] = {'R1': g1, 'R2': [x[2] for x in h], 'R3': g3, 'd': d1, 'band': b1, 'band3': b3,
                                          'pct': 100 * d1 / amean, 'A': rd['A_cell'], 'B': rd['B_cell'],
                                          'npairs': rd['n_pairs'], 'ph': c, 'diffs': rd['diffs'],
                                          'v': rd['v']}
                    else:
                        VERD[(row, z)]['R1_all_orig'] = g1
                        VERD[(row, z)]['d_all_orig'] = d1
                        VERD[(row, z)]['band_all_orig'] = b1
                        VERD[(row, z)]['R2_all_orig'] = [x[2] for x in h]
                        VERD[(row, z)]['ph_all_orig'] = c
        rd = readings(slot, span)
        d1, b1, g1 = rd['R1']
        avals = [x for x in (rd['v'][k] for k in ('A1', 'A2', 'A3', 'A4')) if x is not None]
        amean = statistics.mean(avals)
        P(f'  frame span (largest end_off_ns, post hoc) us: {show(rd["v"])}')
        hs = [fnum(x[0], '+.2f') + ' vs ' + fnum(x[1], '.2f') + ' ' + x[2] for x in rd['R2']]
        P(f'     R1: median(B-A) {d1:+.2f} us ({100 * d1 / amean:+.2f} %), band {b1:.2f} -> {g1}; R2 ' + ' | '.join(hs))

P('')
P('######## gate summary (per-frame median, the gated statistic)')
for (row, z), v in VERD.items():
    P(f'{row:28s} zone {z:2d} {ZN[z]:22s} slot rule: d {v["d"]:+7.2f} us ({v["pct"]:+.2f} %) band {v["band"]:6.2f} '
      f'R1 {v["R1"]}; R2 {v["R2"]}; R3 {v["R3"]} (band {v["band3"]:.2f}) | all originals: d {v.get("d_all_orig", 0):+.2f} '
      f'band {v.get("band_all_orig", 0):.2f} R1 {v.get("R1_all_orig")}')
open(os.path.join(L.HERE, 'dm1.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
import json  # noqa: E402
json.dump({f'{row}|{z}': v for (row, z), v in VERD.items()}, open(os.path.join(L.HERE, 'dm1.json'), 'w'), indent=1)
