import json, sys, hashlib
# usage: exes.py <cargo json log>  -> prints bench-name  executable  fresh  sha256
for line in open(sys.argv[1], encoding='utf-8'):
    line = line.strip()
    if not line.startswith('{'): continue
    m = json.loads(line)
    if m.get('reason') != 'compiler-artifact': continue
    if m['target']['kind'] != ['bench']: continue
    exe = m.get('executable')
    h = hashlib.sha256(open(exe, 'rb').read()).hexdigest()[:16] if exe else None
    print(m['target']['name'], exe, 'fresh' if m['fresh'] else 'built', h)
