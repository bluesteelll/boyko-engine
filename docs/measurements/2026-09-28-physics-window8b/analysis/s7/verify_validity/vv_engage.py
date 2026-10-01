"""Independent recomputation of the S7 pre-flight (gate/s7/): exits/poses/void on the 28 processes, cmp P vs T,
and the engagement receipt from the armed SUMMARY w8s object (lanes sum = lanes_mean x waves; bound
8 x color_scopes + (W+1) x setup_steps (cut.md amended section), and the unamended section-5 bound 8 x scopes + 16 x
setup_steps beside it). Also: disarmed timed SUMMARY fields that differ between P and T (identity evidence)."""
import json
import os
import glob

W = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b'
G = W + '/gate/s7'
FIX = W + '/gate/fixtures'
OUT = []


def say(s):
    OUT.append(s)
    print(s)


def summ(d):
    t = open(os.path.join(d, 'stdout.txt'), 'rb').read().decode('utf-8', 'replace')
    ss = [json.loads(l[8:]) for l in t.splitlines() if l.startswith('SUMMARY ')]
    return ss[0] if len(ss) == 1 else None


fix = {'JT': open(FIX + '/JT500.pose', 'rb').read(), 'JA': open(FIX + '/JA500.pose', 'rb').read()}
ok_all = True
for d in sorted(glob.glob(G + '/*-*-W*')):
    name = os.path.basename(d)
    b, cfg, wpart = name.split('-')[0], name.split('-')[1], name.split('-')[2]
    w = int(wpart[1:])
    armed = name.endswith('armed')
    s = summ(d)
    a = s['args']
    probs = []
    if s.get('void_steps') != 0 or s.get('pose_hash') != '0x30c5438bc6ad9ffa' or s.get('expect_pose') != 'match':
        probs.append('pose/void')
    if s.get('workers') != w or bool(s.get('armed')) != armed:
        probs.append('workers/armed')
    if a[a.index('--steps') + 1] != '500' or a[a.index('--window') + 1] != '0..500':
        probs.append('steps/window')
    want_cfg = 'default' if cfg == 'JT' else 'a'
    if a[a.index('--cfg') + 1] != want_cfg:
        probs.append('cfg')
    pb = open(os.path.join(d, 'pose.bin'), 'rb').read()
    if pb != fix[cfg]:
        probs.append('pose bytes')
    if probs:
        ok_all = False
    say(f'{name}: W{w} armed {armed} steps {a[a.index("--steps") + 1]} pose {s.get("pose_hash")} {s.get("expect_pose")} '
        f'void {s.get("void_steps")} {"OK" if not probs else probs}')
say(f'pre-flight processes all OK: {ok_all}')
for cfg in ('JT', 'JA'):
    for w in (1, 2, 4, 8, 16):
        pa = open(f'{G}/P-{cfg}-W{w}/pose.bin', 'rb').read()
        pt = open(f'{G}/T-{cfg}-W{w}/pose.bin', 'rb').read()
        say(f'cmp P vs T {cfg} W{w}: equal {pa == pt}')

say('\n## Engagement (armed, 500 steps)')
eng = {}
for b in ('P', 'T'):
    for cfg in ('JT', 'JA'):
        for w in (8, 16):
            s = summ(f'{G}/{b}-{cfg}-W{w}-armed')
            x = s['w8s']
            lanes = x['lanes_mean'] * x['waves']
            bound = 8 * x['color_scopes'] + (w + 1) * x['setup_steps']
            bound5 = 8 * x['color_scopes'] + 16 * x['setup_steps']
            eng[(b, cfg, w)] = x
            say(f'{b} {cfg} W{w}: waves {x["waves"]} = scopes {x["color_scopes"]} + setup_steps {x["setup_steps"]}: '
                f'{x["waves"] == x["color_scopes"] + x["setup_steps"]}; lanes_mean {x["lanes_mean"]:.4f} -> lanes sum '
                f'{lanes:.0f}; bound (W+1) {bound} within {lanes <= bound}; bound (16) {bound5} within {lanes <= bound5}; '
                f'tasks {x["color_tasks"]} setup_tasks {x["setup_tasks"]}; overflow {x.get("overflow")}; '
                f'drops {s.get("drops_total")}; workers {s["threads"]["pool_workers"]}')
for cfg in ('JT', 'JA'):
    for w in (8, 16):
        p, t = eng[('P', cfg, w)], eng[('T', cfg, w)]
        eq = all(p[k] == t[k] for k in ('color_scopes', 'color_tasks', 'setup_steps', 'setup_tasks'))
        say(f'{cfg} W{w}: P = T on scopes/tasks/setup: {eq}')
say('receipt: T within at W16 on JT and JA: '
    + str(all(eng[('T', c, 16)]['lanes_mean'] * eng[('T', c, 16)]['waves'] <= 8 * eng[('T', c, 16)]['color_scopes'] + 17 * eng[('T', c, 16)]['setup_steps'] for c in ('JT', 'JA')))
    + '; P above at W16 on JT and JA: '
    + str(all(eng[('P', c, 16)]['lanes_mean'] * eng[('P', c, 16)]['waves'] > 8 * eng[('P', c, 16)]['color_scopes'] + 17 * eng[('P', c, 16)]['setup_steps'] for c in ('JT', 'JA'))))
dbg = open('C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/s7-omega2/tr1/t_s7_debug.txt', encoding='utf-8', errors='replace').read().splitlines()
say('engine read lines: ' + ' | '.join(f'{i + 1}: {l.strip()}' for i, l in enumerate(dbg) if 'physical' in l))
say('t_s7_debug commit line: ' + ' | '.join(l.strip() for l in dbg[:3]))

say('\n## Timed disarmed SUMMARY: fields that differ between s7p and s7t within a (row, W) (identity evidence)')
recs = [json.loads(l) for l in open(W + '/raw/runs.jsonl', encoding='utf-8') if l.strip()]
tim = [r for r in recs if r.get('block') == 'S7-AB' and 'row' in r and r.get('attempt') in ('original', 'rerun')]
def flat(d, pre=''):
    out = {}
    for k, v in d.items():
        if isinstance(v, dict):
            out.update(flat(v, pre + k + '.'))
        else:
            out[pre + k] = v
    return out
skip = ('args', 'label', 'window_mean_ns', 'window_steps_per_s', 'ticks_per_ns')
diffkeys = {}
for rid, w in (('S7-JT', 16), ('S7-JA', 16), ('S7-JT', 8), ('S7-JA', 8), ('S7-JT-W1', 1)):
    vals = {}
    for key in ('s7p', 's7t'):
        for r in tim:
            if r['row'] == rid and r['W'] == w and r['binary'] == key:
                f = flat(summ(r['cwd']))
                for k, v in f.items():
                    if k in skip:
                        continue
                    vals.setdefault(key, {}).setdefault(k, set()).add(json.dumps(v))
    ks = sorted(set(vals['s7p']) | set(vals['s7t']))
    d = [k for k in ks if vals['s7p'].get(k) != vals['s7t'].get(k)]
    say(f'{rid}@W{w}: {len(ks)} SUMMARY fields; differing P vs T: ' + '; '.join(
        f'{k} P {sorted(vals["s7p"].get(k, []))[:4]} T {sorted(vals["s7t"].get(k, []))[:4]}' for k in d))
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'vv_engage_out.txt'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(OUT) + '\n')
