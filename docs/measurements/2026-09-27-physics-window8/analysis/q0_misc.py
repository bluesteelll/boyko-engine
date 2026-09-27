from q2_identity import *

OUT.clear()
P('# section 0: receipts and witness over the used set, by block and pass')
allp = json.load(open(os.path.join(L.HERE, 'proc8.json')))['procs']
for blk in ('W8S-A', 'W8S-R', 'micro', 'DM1', 'P-jolt56-prof'):
    for ps_ in (0, 1):
        ps = [p for p in PROCS if p['block'] == blk and p['pass'] == ps_]
        if not ps:
            continue
        wit = sorted(p['others_busy_pct'] for p in ps)
        rb = sorted([p['receipt_before'] for p in ps] + [p['receipt_after'] for p in ps])
        P(f'{blk} p{ps_}: n {len(ps)}; witness median {statistics.median(wit):.2f} p90 {wit[int(0.9 * len(wit))]:.2f} max {wit[-1]:.2f}; receipts median {statistics.median(rb):.2f} max {rb[-1]:.2f}')
tops = {}
for p in allp:
    for n, c in p['witness_top'][:1]:
        tops[n] = tops.get(n, 0) + 1
P('witness top-1 counts over all processes: ' + str(sorted(tops.items(), key=lambda x: -x[1])[:8]))
P('\n# reuse-off bridge and other rows, disarmed wall [0,500) and [100,500), block A')
for row in ('J-T', 'J-T-off', 'J-A', 'J-Son'):
    for W in (1, 8, 16):
        c = C(row, W, wall(), ['W8S-A'])
        c2 = C(row, W, wall('100..500'), ['W8S-A'])
        if c:
            P(f'{row} W{W}: [0,500) {L.fc(c)}; [100,500) {c2["median"]:.4f}')
    P('')
for W in (1, 8, 16):
    a = C('J-T-off', W, wall(), ['W8S-A'])
    b = C('J-T', W, wall(), ['W8S-A'])
    P(f'reuse on vs off (J-T/J-T-off) W{W}: {L.fcmp(L.cmp_(a, b))}')
P('\n# armed vs disarmed per row and W (A/A1-style perturbation), [0,500)')
for row in ('J-T', 'J-T-off', 'J-A', 'J-Son'):
    for W in (1, 8, 16):
        d = C(row, W, wall(), ['W8S-A'])
        a = C(row + '-a', W, wall(), ['W8S-A'])
        if d and a:
            P(f'{row}-a/{row} W{W}: {L.fcmp(L.cmp_(d, a))}')
P('\n# armed J-T-off spans [100,500) W1/W8/W16 (the bridge to window 7 P4)')
for W in (1, 8, 16):
    m = medterms('J-T-off-a', W, ['W8S-A'], '100..500')
    P(f'W{W}: T {m["T"]:.4f} bp {m["bp"]:.4f} np {m["np"]:.4f} (disp {m["np_disp"]:.4f}) graph {m["graph"]:.4f} setup {m["setup"]:.4f} (pc {m["sb_pc"]:.4f}) warm {m["warm"]:.4f} wide {m["wide"]:.4f} narrow {m["narrow"]:.4f} integ {m["integ_grp"]:.4f} gather {m["gather"]:.4f} g {m["g"]:.4f} u {m["u"]:.4f} r {m["r"]:.4f} waves {m["waves"]:.2f} manifolds {m["manifolds"]:.2f}')
P('\n# J-A armed spans [100,500) W1/W8/W16')
for W in (1, 8, 16):
    m = medterms('J-A-a', W, ['W8S-A'], '100..500')
    P(f'W{W}: T {m["T"]:.4f} bp {m["bp"]:.4f} np {m["np"]:.4f} graph {m["graph"]:.4f} setup {m["setup"]:.4f} warm {m["warm"]:.4f} wide {m["wide"]:.4f} narrow {m["narrow"]:.4f} integ {m["integ_grp"]:.4f} g {m["g"]:.4f} r {m["r"]:.4f}')
save('q0.txt')
