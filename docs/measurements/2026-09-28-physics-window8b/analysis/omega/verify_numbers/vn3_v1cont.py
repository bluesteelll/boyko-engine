"""v1 continuity: 8b omega-v1-cont vs window 8 omega-b rows, both recomputed from their raw/ (window 8's N5 method)."""
import json
import os
import statistics
import sys
sys.dont_write_bytecode = True
import vn_lib as L

# ---- 8b
procs = L.load_procs()
slots, used, dropped = L.select(procs)
v1 = [p for p in used if p['block'] == 'omega-v1-cont']

# ---- window 8 (its own rows / pin / slot rule)
W8RAW = os.path.join(L.W8, 'raw')
W8ROWS = {r['id']: r for r in json.load(open(os.path.join(L.W8, 'rows8.json'), encoding='utf-8'))['rows']}
W8PIN = None
for ln in open(os.path.join(L.W8, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if 'omega_b_region_226bd99e.exe' in ln:
        W8PIN = ln.split()[0]
w8 = []
for ln in open(os.path.join(W8RAW, 'runs.jsonl'), encoding='utf-8'):
    r = json.loads(ln)
    if r.get('pass_done') or r.get('kind') != 'micro' or r.get('attempt') == 'warmup':
        continue
    if not r['row'].startswith('omega-b-'):
        continue
    summ, cal = L.parse(r['cwd'])
    row = W8ROWS[r['row']]
    why = []
    if r.get('exit') != 0 or r.get('exe_sha256') != W8PIN or len(summ) != row['expect_summaries']:
        why.append('invalid')
    for s in summ:
        if s.get('bench') != row['expect_bench'] or s.get('route') != row['expect_route'] or s.get('stages') != row['expect_stages']:
            why.append('field')
    r['_summ'], r['_valid'], r['_clean'] = summ, why, L.clean_why(r)
    w8.append(r)
w8slots = {}
for r in w8:
    w8slots.setdefault((r['pass'], r['round'], r['row']), []).append(r)
w8used = []
for k in sorted(w8slots):
    cand = [p for p in w8slots[k] if p['attempt'] == 'original'] + [p for p in w8slots[k] if p['attempt'] == 'rerun']
    pick = next((p for p in cand if not p['_valid'] and not p['_clean']), None)
    if pick:
        w8used.append(pick)
print('window 8 omega-b processes', len(w8), 'slots', len(w8slots), 'used', len(w8used), 'pin', W8PIN[:8])


def rcell(group, route, st, P, key='region_ns_median'):
    xs = []
    for p in group:
        if p['row'] != ('omega-b-s%d-%s' % (st, route)) and p['row'] != ('omega1-s%d-%s' % (st, route)):
            continue
        for s in p['_summ']:
            if s['participants'] == P:
                xs.append(s[key])
    return L.cell(xs)


def paired(group, route, P):
    d = {}
    for p in group:
        for st in (36, 72):
            if p['row'] in ('omega-b-s%d-%s' % (st, route), 'omega1-s%d-%s' % (st, route)):
                for s in p['_summ']:
                    if s['participants'] == P:
                        d.setdefault((p['pass'], p['round']), {})[st] = s['region_ns_median']
    xs = [(v[72] - v[36]) / 36 for v in d.values() if 36 in v and 72 in v]
    return L.cell(xs) if xs else None


for route in ('worker', 'external'):
    for P in (2, 4, 8, 16):
        a36, a72 = rcell(w8used, route, 36, P), rcell(w8used, route, 72, P)
        b36, b72 = rcell(v1, route, 36, P), rcell(v1, route, 72, P)
        sa = (a72['med'] - a36['med']) / 36
        sb = (b72['med'] - b36['med']) / 36
        c36 = L.cmp2(a36, b36, True)
        c72 = L.cmp2(a72, b72, True)
        n36 = L.cmp2(a36, b36, False)
        n72 = L.cmp2(a72, b72, False)
        pa, pb = paired(w8used, route, P), paired(v1, route, P)
        print('%-8s P%-2d w8 %.0f/%.0f (K %d/%d) 8b %.0f/%.0f (K %d/%d) | slope w8 %.1f 8b %.1f ratio %.3f [8b worst pairing %.0f..%.0f] | region36 %s (no guard %s) region72 %s (no guard %s) | paired w8 %s / 8b %s'
              % (route, P, a36['med'], a72['med'], a36['K'], a72['K'], b36['med'], b72['med'], b36['K'], b72['K'],
                 sa, sb, sb / sa, (b72['min'] - b36['max']) / 36, (b72['max'] - b36['min']) / 36, L.flags(c36),
                 L.flags(n36), L.flags(c72), L.flags(n72),
                 '%.0f (n %d)' % (pa['med'], pa['K']) if pa else '-', '%.0f (n %d)' % (pb['med'], pb['K']) if pb else '-'))
