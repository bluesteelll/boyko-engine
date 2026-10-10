import json, sys, collections
recs = [json.loads(l) for l in open(sys.argv[1], encoding='utf-8')]
avgs = []; tops = collections.Counter(); builds = collections.Counter()
for r in recs:
    for k in ('receipt_before', 'receipt_after'):
        rc = r.get(k)
        if not rc: continue
        avgs.append(rc['cpu_avg'])
        for t in rc['top5'][:3]:
            if t['pct_of_machine'] >= 1.0: tops[t['name']] += 1
        for b in rc['build_procs_busy']: builds[b] += 1
avgs = [a for a in avgs if a is not None]
avgs.sort()
print('receipts', len(avgs), 'cpu_avg min/median/max', avgs[0], avgs[len(avgs)//2], avgs[-1], 'count > 5%:', sum(a > 5 for a in avgs))
print('processes >=1% of machine among top3 (receipt count):', tops.most_common(12))
print('build processes busy:', dict(builds))
