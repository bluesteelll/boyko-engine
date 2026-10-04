"""v0: C4-AB processes, independent validity vs the driver, clean, slot rule, K per pass-cell vs driver passcell."""
import collections
import vlib as L

o = L.Out()
recs = L.records()
blk = 'C4-AB'
done = sorted({(r['pass']) for r in recs if r.get('pass_done') and r.get('block') == blk})
o(f'C4-AB pass_done passes: {done}')
voided = [r for r in recs if r.get('voided_pass') and r.get('block') == blk]
o(f'voided pass records: {len(voided)}')
procs, used, dropped, slots = L.select(blk, recs)
att = collections.Counter(p['attempt'] for p in procs)
o(f'timed processes {len(procs)}: {dict(att)}; warm-ups (excluded) '
  f"{sum(1 for r in recs if r.get('block') == blk and r.get('attempt') == 'warmup')}")
inv = [p for p in procs if not p['valid']]
o(f'invalid (mine): {len(inv)}')
for p in inv:
    o(f"  INVALID {p['row']}#{p['binary']}@W{p['W']} p{p['pass_']} seq{p['seq']} {p['attempt']}: {p['valid_why']}")
dis = [p for p in procs if bool(p['driver_valid']) != p['valid']]
o(f'disagree with driver valid flag: {len(dis)} of {len(procs)}')
unclean = [p for p in procs if not p['clean']]
o(f'unclean: {len(unclean)} (by pass: {dict(collections.Counter(p["pass_"] for p in unclean))})')
o(f'slots {len(slots)}; used {len(used)} (reruns used {sum(1 for p in used if p["attempt"] == "rerun")}); dropped {len(dropped)}'
  f' (by pass {dict(collections.Counter(k[0] for k, _ in dropped))})')
# per pass
for ps in (0, 1, 2):
    pp = [p for p in procs if p['pass_'] == ps]
    ori = [p for p in pp if p['attempt'] == 'original']
    o(f"pass {ps}: {len(pp)} processes ({len(ori)} originals, {len(pp) - len(ori)} re-runs); unclean {sum(1 for p in pp if not p['clean'])}; "
      f"originals unclean {sum(1 for p in ori if not p['clean'])}; witness median originals "
      f"{sorted(p['rec'].get('others_busy_pct', 0) for p in ori)[len(ori) // 2]:.2f} %")
# K per pass-cell vs driver
K = collections.Counter((p['pass_'], p['row'], p['binary'], p['W']) for p in used)
pcs = [r for r in recs if r.get('passcell') and r.get('block') == blk]
agree = 0
short = []
for r in pcs:
    k = (r['pass'], r['pc_row'], r['pc_binary'], r['pc_W'])
    mine = K.get(k, 0)
    if mine == r['k_clean']:
        agree += 1
    else:
        o(f'  K DISAGREE {k}: mine {mine} driver {r["k_clean"]}')
    if mine < 3:
        short.append((k, mine))
o(f'pass-cells {len(pcs)}; my K == driver k_clean on {agree}/{len(pcs)}')
o(f'short pass-cells (K<3): {len(short)}; by pass {dict(collections.Counter(k[0] for k, _ in short))}')
full_p0 = sorted(f'{k[1]}#{k[2]}@W{k[3]}' for k in K if k[0] == 0 and K[k] >= 3)
o(f'p0 full pass-cells: {full_p0}')
for (k, m) in sorted(short):
    o(f'  SHORT p{k[0]} {k[1]}#{k[2]}@W{k[3]} K={m}')
# hashes of used
hs = collections.Counter((p['kind'], p.get('pose_hash')) for p in used)
o(f'used hashes: {dict(hs)}')
o(f"used jolt processes: {sum(1 for p in used if p['kind'] == 'jolt')}")
o(f"void_steps over used runner: {collections.Counter(p.get('void') for p in used if p['kind'] == 'runner')}")
o(f"sleeping over used runner: {collections.Counter(p.get('sleeping') for p in used if p['kind'] == 'runner')}")
# top other on unclean
top = collections.Counter()
for p in unclean:
    t5 = p['rec'].get('others_top5') or []
    if t5:
        top[t5[0]['name']] += 1
o(f'top other process on unclean runs: {top.most_common(6)}')
o.save('v0_validity.txt')
