import json, os
W4 = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4'
def walk(root, base):
    out = []
    for dp, dn, fn in os.walk(root):
        if os.path.basename(dp) == base and 'estimates.json' in fn:
            e = json.load(open(os.path.join(dp, 'estimates.json')))
            rel = os.path.relpath(dp, root).replace(os.sep, '/')
            out.append((rel[:-len('/' + base)], e['median']['point_estimate']))
    return sorted(out)
print('scene group (rehearsal r1, real criterion settings, loaded machine):')
for c, v in walk(W4 + '/rehearsal/crit', 'r1'):
    print(f'  {c}: {v/1e3:.1f} us')
print('maintenance + 2 churn (G4_TEST rehearsal, 0.2 s warm-up / 0.5 s measurement, loaded machine):')
for c, v in walk(W4 + '/test/g4', 'k1'):
    print(f'  {c}: {v/1e3:.1f} us')
