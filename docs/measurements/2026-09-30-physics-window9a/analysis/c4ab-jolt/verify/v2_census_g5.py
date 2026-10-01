"""v2: equal-work census from C4-G5 C4-JD-armed#tip W1/W8 (used processes), G5 'recorded', armed void steps."""
import collections, os
import vlib as L

o = L.Out()
recs = L.records()
g5 = [r for r in recs if r.get('block') == 'C4-G5' and 'row' in r and r.get('attempt') in ('original', 'rerun')]
done = sorted(r['pass'] for r in recs if r.get('pass_done') and r.get('block') == 'C4-G5')
o(f'C4-G5 pass_done: {done}; processes {len(g5)} ({collections.Counter(r["attempt"] for r in g5)}); driver-invalid {sum(1 for r in g5 if not r.get("valid"))}')
pcs = [r for r in recs if r.get('passcell') and r.get('block') == 'C4-G5']
o(f'C4-G5 pass-cells {len(pcs)}; k_clean==3: {sum(1 for r in pcs if r["k_clean"] >= 3)}')
# slot rule on G5 using driver validity + my clean rule (G5 rows differ; validity re-check not needed for census values)
slots = {}
for r in g5:
    slots.setdefault((r['pass'], r['seq']), []).append(r)
used = []
for k, ps in sorted(slots.items()):
    orig = [p for p in ps if p['attempt'] == 'original']
    rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p.get('rerun_no', 0))
    pick = next((c for c in orig + rer if c.get('valid') and not L.clean_why(c)), None)
    if pick:
        used.append(pick)
o(f'G5 used slots {len(used)} of {len(slots)}')
# armed void steps on EVERY armed process (not only used)
armed = [r for r in g5 if r['row'] in ('C4-JD-armed', 'C4-JDap-armed')]
vs = collections.Counter()
for r in armed:
    s = L.parse_summary(L._read_text(os.path.join(r['cwd'], 'stdout.txt')))
    vs[(r['row'], s.get('void_steps') if s else None)] += 1
o(f'armed processes {len(armed)}: void_steps census {dict(vs)}')
# census
cen = []
for r in used:
    if r['row'] != 'C4-JD-armed':
        continue
    hdr, rows = L.read_csv(os.path.join(r['cwd'], 'run.csv'))
    ip, im, iman = hdr.index('phys_np_points'), hdr.index('phys_np_manifolds'), hdr.index('manifolds')
    sub = rows[100:500]
    assert [int(x[0]) for x in sub] == list(range(100, 500))
    pts = sum(float(x[ip]) for x in sub) / 400
    npm = sum(float(x[im]) for x in sub) / 400
    man = sum(float(x[iman]) for x in sub) / 400
    s = L.parse_summary(L._read_text(os.path.join(r['cwd'], 'stdout.txt')))
    cen.append((r['W'], r['pass'], pts, npm, man, s.get('pose_hash')))
o(f'C4-JD-armed used processes: {len(cen)}; W split {collections.Counter(c[0] for c in cen)}')
o(f'distinct (points, np_manifolds, manifolds, hash): {sorted(set((round(c[2], 4), round(c[3], 4), round(c[4], 4), c[5]) for c in cen))}')
Po = cen[0][2]
Mo = cen[0][3]
Mj, Pj = 8489.0, 31111.958
vro = Po * 3 * 12
vrj = (Pj + 3 * Mj) * 10
prj = 2 * Pj
o(f'ours: points {Po:.3f} manifolds {Mo:.3f} -> velocity row-iterations {vro:,.1f} (DOSSIER 595,935)')
o(f'Jolt (DOSSIER): velocity {vrj:,.1f} (565,790) + position {prj:,.1f} (62,224)')
o(f'Jolt/ours: velocity rows {vrj / vro:.4f}; all rows {(vrj + prj) / vro:.4f}; manifolds {Mj / Mo:.4f}')
# ns per unit table from C4-AB cells [100,500)
procs, usedAB, dropped, _ = L.select('C4-AB', recs)
for W in (1, 2, 4, 8, 16):
    jd = L.cell([p['m100_500'] for p in usedAB if p['row'] == 'C4-JD' and p['binary'] == 'tip' and p['W'] == W])['median']
    jo = L.cell([p['m100_500'] for p in usedAB if p['row'] == 'C4-jolt56' and p['W'] == W])['median']
    o(f'W{W}: [100,500) ours {jd:.4f} Jolt {jo:.4f} ms | ns/manifold ours {jd * 1e6 / Mo:.1f} Jolt {jo * 1e6 / Mj:.1f} | '
      f'ns/vrow ours {jd * 1e6 / vro:.2f} Jolt {jo * 1e6 / vrj:.2f}')
o.save('v2_census_g5.txt')
