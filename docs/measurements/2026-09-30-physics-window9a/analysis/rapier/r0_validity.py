"""r0: validity, cleanliness, slot census, pass-cell K, block times and receipts, our row census (C4-JD-armed#tip),
exe hashes on disk. Rapier group of window 9a."""
import collections
import csv
import hashlib
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_rp as L  # noqa: E402

o = L.Out()
recs = L.all_records()
o('closed passes: ' + ', '.join(sorted(f'{b}-p{p}' for _, b, p, _a in L.closed_passes(recs))))
blocks = {'C4-RAPIER': None, 'C4-AB': {'C4-JT', 'C4-JD', 'C4-jolt56'}, 'C4-G5': {'C4-JD-armed'}}
SEL = {}
for b, rows in blocks.items():
    procs, used, dropped = L.select(b, recs, rows)
    SEL[b] = (procs, used, dropped)
    n_inv = sum(1 for p in procs if p['valid_why'])
    agree_v = sum(1 for p in procs if (not p['valid_why']) == p['driver_valid'])
    agree_c = sum(1 for p in procs if (not p['clean_why']) == p['driver_clean'])
    agree_m = sum(1 for p in procs if p['driver_agree'])
    o(f'{b} rows {sorted(rows) if rows else "all"}: processes {len(procs)} (original {sum(p["attempt"] == "original" for p in procs)}, '
      f're-run {sum(p["attempt"] == "rerun" for p in procs)}); invalid (mine) {n_inv}; validity agrees with driver {agree_v}/{len(procs)}; '
      f'clean agrees with driver {agree_c}/{len(procs)}; window means agree with driver cols (<= 1 ns) {agree_m}/{len(procs)}; '
      f'unclean {sum(1 for p in procs if p["clean_why"])}; slots used {len(used)}, dropped {len(dropped)}')
    for p in procs:
        if p['valid_why']:
            o(f'  INVALID {p["row"]}#{p["binary"]}@W{p["W"]} p{p["pass"]} r{p["round"]} {p["attempt"]}: {p["valid_why"]}')
    why = collections.Counter()
    for p in procs:
        for w in p['clean_why']:
            why[w.split(' ')[0]] += 1
    o(f'  unclean causes: {dict(why)}')
    for k, ps in dropped:
        o(f'  DROPPED slot {k}: attempts {[(p["attempt"], p["rerun_no"], p["clean_why"]) for p in ps]}')

o()
o('pass-cell K (clean valid slots per pass) for the cells this group reads; driver passcell record k_clean in brackets')
pcs = {(r['block'], r['pass'], r['pc_row'], r['pc_binary'], r['pc_W']): r for r in recs if r.get('passcell')}
for b, (procs, used, dropped) in SEL.items():
    cells = sorted({(p['row'], p['binary'], p['W']) for p in procs})
    for (row, binary, W) in cells:
        ks = []
        for ps in (0, 1, 2):
            k = sum(1 for p in used if (p['row'], p['binary'], p['W'], p['pass']) == (row, binary, W, ps))
            d = pcs.get((b, ps, row, binary, W))
            ks.append(f'{k}[{d["k_clean"] if d else "-"}]')
        flag = '' if all(x.startswith('3') for x in ks) else '   <- SHORT pass-cell'
        o(f'  {b:10s} {row:12s} {binary:6s} W{W:<3d} K p0/p1/p2 = {" / ".join(ks)}{flag}')

o()
o('block and pass times (first process start .. last process end, used + unused)')
for b, (procs, used, dropped) in SEL.items():
    for ps in (0, 1, 2):
        pp = [p['rec'] for p in procs if p['pass'] == ps]
        if pp:
            o(f'  {b}-p{ps}: {min(r["start"] for r in pp)[11:19]} .. {max(r["end"] for r in pp)[11:19]}')

o()
o('receipts of the USED processes (medians): machine busy before / after (%), witness others_busy (%)')
for b, (procs, used, dropped) in SEL.items():
    for ps in (0, 1, 2, 'all'):
        uu = [p['rec'] for p in used if ps == 'all' or p['pass'] == ps]
        if not uu:
            continue
        rb = statistics.median(r['receipt_before']['cpu_avg'] for r in uu)
        ra = statistics.median(r['receipt_after']['cpu_avg'] for r in uu)
        wi = statistics.median(r['others_busy_pct'] for r in uu)
        o(f'  {b}-p{ps}: n {len(uu)}  before {rb:.2f}  after {ra:.2f}  witness {wi:.2f}')

o()
o('our row-iteration census: 36 * mean(phys_np_points) over the window, C4-JD-armed#tip (G5), every used process')
cen = collections.defaultdict(set)
for p in SEL['C4-G5'][1]:
    c = list(csv.DictReader(open(os.path.join(p['rec']['cwd'], 'run.csv'), encoding='utf-8')))
    pts = [float(x['phys_np_points']) for x in c]
    nm = [float(x['phys_np_manifolds']) for x in c]
    for w in ((0, 100), (100, 500), (0, 500)):
        cen[('rows36', w, p['W'])].add(round(36 * statistics.fmean(pts[w[0]:w[1]]), 3))
        cen[('Np', w, p['W'])].add(round(statistics.fmean(pts[w[0]:w[1]]), 4))
        cen[('Nm', w, p['W'])].add(round(statistics.fmean(nm[w[0]:w[1]]), 4))
for k in sorted(cen, key=str):
    o(f'  {k[0]:6s} [{k[1][0]},{k[1][1]}) W{k[2]}: {sorted(cen[k])} (distinct values over {sum(1 for p in SEL["C4-G5"][1] if p["W"] == k[2])} processes)')
o('  pins: Rapier RP-D rows 303,003 [0,100) / 354,674 [100,500); RP-M 935,729 / 1,071,107 (window9a_rows.md rows_pin)')

o()
o('exe sha256 on disk now vs pin')
for key in ('tip', 'j56', 'rs8', 'rs4'):
    exe = L.BINS[key]['exe']
    path = exe if os.path.isabs(exe) else os.path.join(L.W9A, exe)
    h = hashlib.sha256(open(path, 'rb').read()).hexdigest()
    o(f'  {key}: {h == L.BINS[key]["sha256_pin"]} ({h[:16]}...)')
json.dump({'census': {f'{k[0]}|{k[1][0]}-{k[1][1]}|W{k[2]}': sorted(v) for k, v in cen.items()}},
          open(os.path.join(L.HERE, 'r0_validity.json'), 'w'), indent=1)
o.save('r0_validity.txt')
