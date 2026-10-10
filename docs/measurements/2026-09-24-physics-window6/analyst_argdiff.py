import json, os
recs = [json.loads(l) for l in open('raw/runs.jsonl')]
seen = {}
for r in recs:
    if r.get('row') in ('C3b-TA-armed', 'G5-TA-tree-a') and r['binary'] == 'tip' and r['W'] == 1 and r['attempt'] == 'original':
        seen.setdefault(r['row'], r)
a, b = seen['C3b-TA-armed'], seen['G5-TA-tree-a']
def norm(xs):
    out = []
    for x in xs:
        i = x.find('raw' + os.sep)
        out.append('<raw>/' + x[i + 4:] if i >= 0 else x)
    return out
print(norm(a['args']))
print(norm(b['args']))
print('cmdline chars', sum(len(x) + 1 for x in a['args']), sum(len(x) + 1 for x in b['args']))
print('config equal:', a['summary']['config'] == b['summary']['config'])
