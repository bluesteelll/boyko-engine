import json, sys
recs = [json.loads(l) for l in open(sys.argv[1], encoding='utf-8')]
for r in recs:
    if r['engine'] == 'boyko' and r['W'] in (1, 8):
        s = r['summary']; c = s['config']
        flags = []
        skip = False
        for a in r['args']:
            if skip: skip = False; continue
            if a in ('--csv', '--pose-out', '--expect-pose', '--label'): skip = True; continue
            flags.append(a)
        print(f"{r['row']}@W{r['W']}: [{' '.join(flags)}] solver={s['solver']} cfg={s['cfg']} bp={c['broadphase']} simd_solve={c['simd_solve']} parallel_solve={c['parallel_solve']} sleeping={c['sleeping']} thr={c['sleep_threshold']} armed={s['armed']} profile={s['profile_name']} canary_ns_set={s['canary_ns'] is not None}")
