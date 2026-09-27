"""Selection and per-process statistics, recomputed from the files. Writes analysis/proc8.json."""
import json
import os
import re
import sys

sys.dont_write_bytecode = True
import lib8 as L

WINS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500)}
recs = L.load_recs()
out = []
problems = []
for r in recs:
    if r.get('attempt') == 'warmup':
        continue
    p = {k: r.get(k) for k in ('block', 'pass', 'round', 'row', 'binary', 'W', 'attempt', 'kind', 'seq', 'start', 'end',
                               'others_busy_pct', 'exit', 'cwd')}
    p['receipt_before'] = (r.get('receipt_before') or {}).get('cpu_avg')
    p['receipt_after'] = (r.get('receipt_after') or {}).get('cpu_avg')
    p['witness_top'] = [(o['name'], o['cpu_s']) for o in (r.get('others_top5') or [])[:2]]
    p['clean_why'] = L.clean(r)
    kind = r['kind']
    if kind == 'runner':
        why, s, c = L.validate_runner(r)
        p['valid_why'] = why
        if c is not None:
            st = {}
            for wn, (a, b) in WINS.items():
                st[wn] = {h: L.mean(v[a:b]) for h, v in c.items() if h not in ('step', 'top_y', 'void')}
            p['stats'] = st
            p['wall_ms'] = {wn: st[wn]['wall_ns'] / 1e6 for wn in WINS}
        if s is not None:
            p['summary'] = {k: s.get(k) for k in ('pose_hash', 'armed', 'canary_ns', 'canary_zone', 'canary_zone_ns',
                                                 'w8s', 'threads', 'pair_classes', 'final_manifolds', 'waves_total',
                                                 'first_frozen_step', 'ticks_per_ns')}
            p['summary']['config'] = s.get('config')
    elif kind in ('jolt', 'joltprof'):
        why, so, frames = L.validate_jolt(r)
        p['valid_why'] = why
        if frames:
            p['wall_ms'] = {wn: sum(frames[a:b]) / (b - a) for wn, (a, b) in WINS.items()}
            p['frames_ms'] = frames
        m = re.search(r'tsc ticks per second ([\d.]+)', so or '')
        p['tsc'] = float(m.group(1)) if m else None
    elif kind == 'micro':
        so = L.read_text(os.path.join(r['cwd'], 'stdout.txt'))
        ss = [json.loads(x[8:]) for x in so.splitlines() if x.startswith('SUMMARY ')]
        row = L.ROWS[r['row']]
        why = []
        if r.get('exit') != 0:
            why.append('exit')
        if not L.sha_ok(r):
            why.append('sha')
        if len(ss) != row['expect_summaries']:
            why.append(f'{len(ss)} summaries')
        for s in ss:
            if s.get('bench') != row['expect_bench'] or s.get('route') != row['expect_route']:
                why.append('bench/route')
            if row['expect_stages'] and s.get('stages') != row['expect_stages']:
                why.append('stages')
        p['valid_why'] = why
        p['summaries'] = ss
    elif kind == 'dm1':
        d = r['cwd']
        txt = L.read_text(os.path.join(d, 'stdout.txt')) + '\n' + L.read_text(os.path.join(d, 'stderr.txt'))
        why = []
        if r.get('exit') != 0:
            why.append('exit')
        if not L.sha_ok(r):
            why.append('sha')
        if 'test result: ok. 1 passed' not in txt:
            why.append('libtest')
        if 'NOT MEASURED' in txt:
            why.append('not measured')
        m = re.search(r'DM1 timing: path=(\w+) res=(\d+)x(\d+) edit_rows=(\d+) grow_at=(\S+) frames=(\d+)', txt)
        row = L.ROWS[r['row']]
        if not m:
            why.append('no DM1 line')
        else:
            path = {'vb': 'VisibilityBuffer', 'deferred': 'Deferred'}[row['path']]
            if m.group(1) != path or f'{m.group(2)}x{m.group(3)}' != row['res'] or int(m.group(4)) != row['edit_rows']:
                why.append('DM1 line mismatch')
            ga = row.get('grow_at')
            if (ga is None and m.group(5) != 'None') or (ga is not None and m.group(5) != f'Some({ga})'):
                why.append('grow_at mismatch')
        art = L.read_text(os.path.join(d, 'artifact.toml'))
        zones = {}
        for blk in re.findall(r'\[\[zone\]\](.*?)(?=\[\[zone\]\]|\Z)', art, re.S):
            kv = dict(re.findall(r'(\w+) = ([^\n]+)', blk))
            zid = int(kv['id'])
            zones[zid] = {k: (float(v) if re.match(r'^-?[\d.]+', v) else v.strip('"')) for k, v in kv.items()}
        need = {'vb': (2, 14, 9), 'deferred': (17,)}[row['path']]
        for z in need:
            if z not in zones or int(zones[z]['n']) != 220:
                why.append(f'zone {z} n')
        p['valid_why'] = why
        p['zones'] = zones
        p['dm1_line'] = m.group(0) if m else None
        p['present_mode'] = (re.search(r'present_mode = "(\w+)"', art) or [None, None])[1]
    out.append(p)
    if p['valid_why']:
        problems.append((p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'], p['attempt'], p['valid_why']))

# the slot rule: original if valid and clean, else its re-run if valid and clean, else dropped
slots = {}
for p in out:
    k = (p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'])
    slots.setdefault(k, []).append(p)
used, dropped = [], []
dm1_seq = {}
for k, ps in slots.items():
    if k[0] == 'DM1':
        for p in ps:  # DM1 cells repeat a key (ABBA): every process is its own slot
            (used if not p['valid_why'] and not p['clean_why'] else dropped).append(p)
        continue
    orig = [p for p in ps if p['attempt'] == 'original']
    rer = [p for p in ps if p['attempt'] == 'rerun']
    pick = None
    for cand in orig + rer:
        if not cand['valid_why'] and not cand['clean_why']:
            pick = cand
            break
    if pick:
        used.append(pick)
    else:
        dropped.append(ps)
for p in used:
    p['used'] = True
print('processes (non-warm-up):', len(out), 'slots:', len(slots), 'used:', len(used), 'dropped slots:', len(dropped))
print('validity problems:', len(problems))
for x in problems:
    print('  ', x)
unclean = [(p['block'], p['pass'], p['round'], p['row'], p['W'], p['attempt'], p['clean_why']) for p in out if p['clean_why']]
print('unclean processes:', len(unclean))
for x in unclean:
    print('  ', x)
json.dump({'procs': out}, open(os.path.join(L.HERE, 'proc8.json'), 'w'))
