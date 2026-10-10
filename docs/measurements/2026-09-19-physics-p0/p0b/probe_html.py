import re, json, sys
def load(path):
    t = open(path, encoding='utf-8', errors='replace').read()
    i = t.index('var cycles_per_second')
    body = t[i:t.index('</script>', i)]
    cps = int(re.search(r'var cycles_per_second = (\d+);', body).group(1))
    thr = body[body.index('var threads = ')+len('var threads = '):body.index('var aggregated = ')].strip().rstrip(';')
    agg = body[body.index('var aggregated = ')+len('var aggregated = '):].strip().rstrip(';')
    def js2json(s):
        s = re.sub(r'(^|[{,\n])\s*([a-z_]+):', r'\1"\2":', s)
        s = re.sub(r',\s*([}\]])', r'\1', s)
        return json.loads(s)
    return cps, js2json(thr), js2json(agg)
if __name__ == '__main__':
    cps, threads, agg = load(sys.argv[1])
    print('cps', cps, 'threads', [ (t['thread_name'], len(t['start'])) for t in threads])
    for i, n in enumerate(agg['name']):
        # depth histogram of this aggregator across threads
        dh = {}
        for t in threads:
            for a, d in zip(t['aggregator'], t['depth']):
                if a == i: dh[d] = dh.get(d, 0) + 1
        print(i, n[:90], 'calls', agg['calls'][i], 'cpf', agg['cycles_per_frame'][i], 'depths', dh)
