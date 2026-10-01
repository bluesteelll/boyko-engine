"""o3: v1 continuity. Window 8b omega-v1-cont (omega2 exe a3adc827, v1 omega-b mode, K = 3 planned, one pass) against
window 8's omega-b rows (omega exe 226bd99e, K = 6), recomputed here from win8/raw with the same selection (micro8b's
expect_* rules on each window's own rows file, exit 0, sha; window 8's clean rule and slot rule). Method = window 8 N5:
slope = (cell median region@72 - cell median region@36) / 36, plus the per-round paired slope. Also omega(W, gap)
8b vs window 8 (post hoc; not pre-registered)."""
import json
import os
import sys

sys.dont_write_bytecode = True
import lib_om as L
import micro8b

O = L.Out()
W8 = os.path.join(os.path.dirname(L.W8B), 'win8')
ROWS8 = {r['id']: r for r in json.load(open(os.path.join(W8, 'rows8.json'), encoding='utf-8'))['rows']}
SUMS8 = {}
for line in open(os.path.join(W8, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS8[name.strip().lstrip('*')] = sha
p8 = []
for line in open(os.path.join(W8, 'raw', 'runs.jsonl'), encoding='utf-8'):
    r = json.loads(line)
    if r.get('pass_done') or r.get('kind') != 'micro':
        continue
    ss, cal = micro8b.parse(L.read_text(os.path.join(r['cwd'], 'stdout.txt')))
    why, _ = micro8b.process_rules(ROWS8[r['row']], ss, cal)
    if r.get('exit') != 0:
        why.append('exit')
    if r.get('exe_sha256') != SUMS8.get('bin/omega_b_region_226bd99e.exe'):
        why.append('sha')
    p8.append({'block': r['block'], 'pass': r['pass'], 'round': r['round'], 'row': r['row'], 'attempt': r.get('attempt'),
               'summaries': ss, 'cal': cal, 'valid_why': why, 'clean_why': L.clean_why(r)})
u8, d8, _ = L.select(p8)
O('# o3 v1 continuity; window 8 micro: %d processes, %d used, %d dropped slots; window 8 exe sha %s' % (
    len([p for p in p8 if p['attempt'] != 'warmup']), len(u8), len(d8), SUMS8.get('bin/omega_b_region_226bd99e.exe')[:12]))
procs = L.all_procs()
u9, d9, _ = L.select(procs)
u9 = [p for p in u9 if p['block'] == 'omega-v1-cont']
O('window 8b omega-v1-cont: %d used (planned 12), dropped slots %d' % (len(u9), len([d for d in d9 if d[0][0] == 'omega-v1-cont'])))


def region(used, row, P, passes=None):
    return [s['region_ns_median'] for p in used if p['row'] == row and (passes is None or p['pass'] in passes)
            for s in p['summaries'] if s['participants'] == P]


def paired(used, r36, r72, P):
    pp = {}
    for p in used:
        if p['row'] in (r36, r72):
            for s in p['summaries']:
                if s['participants'] == P:
                    pp.setdefault((p['pass'], p['round']), {})[s['stages']] = s['region_ns_median']
    return L.cell([(v[72] - v[36]) / 36 for v in pp.values() if 36 in v and 72 in v])


def cmp_any(a, b):
    """cmp_ with the K >= 3 guard waived (K >= 2), for the sensitivity column."""
    c = L.cmp_(a, b)
    if c:
        e = abs(c['ratio'] - 1)
        for k in ('r', 'i', 's'):
            c['cl_' + k] = a['K'] >= 2 and b['K'] >= 2 and e > c['bar_' + k]
    return c


for route in ('worker', 'external'):
    O('\n## omega_b v1, route %s (ns); window 8 rows omega-b-s{36,72}-%s, 8b rows omega1-s{36,72}-%s' % (route, route, route))
    for P in (2, 4, 8, 16):
        a36, a72 = L.cell(region(u8, 'omega-b-s36-' + route, P)), L.cell(region(u8, 'omega-b-s72-' + route, P))
        b36, b72 = L.cell(region(u9, 'omega1-s36-' + route, P)), L.cell(region(u9, 'omega1-s72-' + route, P))
        s8 = (a72['median'] - a36['median']) / 36
        s9 = (b72['median'] - b36['median']) / 36
        lo9, hi9 = (b72['min'] - b36['max']) / 36, (b72['max'] - b36['min']) / 36
        lo8, hi8 = (a72['min'] - a36['max']) / 36, (a72['max'] - a36['min']) / 36
        pr8, pr9 = paired(u8, 'omega-b-s36-' + route, 'omega-b-s72-' + route, P), paired(u9, 'omega1-s36-' + route, 'omega1-s72-' + route, P)
        O('  P%d: w8 region36 %s; region72 %s' % (P, L.fc(a36, 0), L.fc(a72, 0)))
        O('       8b region36 %s; region72 %s' % (L.fc(b36, 0), L.fc(b72, 0)))
        O('       slope w8 %.1f [worst pairing %.1f..%.1f] | 8b %.1f [worst pairing %.1f..%.1f] | 8b/w8 %.3f | paired per-round slope w8 %s | 8b %s' % (
            s8, lo8, hi8, s9, lo9, hi9, s9 / s8, L.fc(pr8, 0), L.fc(pr9, 0)))
        for st, a, b in ((36, a36, b36), (72, a72, b72)):
            c, c2 = L.cmp_(a, b), cmp_any(a, b)
            O('       region%d 8b vs w8: %s (K>=3 guard) | guard waived %s' % (st, L.fcmp(c, 0), L.yn(c2)))
        c, c2 = L.cmp_(pr8, pr9), cmp_any(pr8, pr9)
        O('       paired slope 8b vs w8: %s | guard waived %s' % (L.fcmp(c, 0), L.yn(c2)))

O('\n## omega(W, gap) 8b (omega-wgap-*) vs window 8 (omega-*), scope_ns_median cells (post hoc)')
for route in ('worker', 'external'):
    for W in (8, 16):
        for gap in (0, 5, 20, 80):
            def sc(used, row):
                return L.cell([s['scope_ns_median'] for p in used if p['row'] == row for s in p['summaries']
                               if s['workers'] == W and s['gap_us'] == gap])
            a, b = sc(u8, 'omega-' + route), sc([p for p in procs if p.get('used')], 'omega-wgap-' + route)
            O('  %s W%d gap %2d: w8 %s | 8b %s | %s' % (route, W, gap, L.fc(a, 0), L.fc(b, 0), L.fcmp(L.cmp_(a, b), 0)))
O.save('o3.txt')
