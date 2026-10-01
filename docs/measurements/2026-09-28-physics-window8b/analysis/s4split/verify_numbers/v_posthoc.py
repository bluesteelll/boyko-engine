"""Post-hoc checks: S7-AB ladder on the TIP file (s7p), S4-AB armed per-wave TIP vs PARENT. Output: v_posthoc.txt."""
import json, os, statistics, ntpath
import vlib as L

OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
TIP_SHA = L.SUMS['runner_tip_16191fda.exe']
recs = [json.loads(l) for l in open(os.path.join(L.RAW, 'runs.jsonl'), encoding='utf-8')]
recs = [r for r in recs if r.get('block') == 'S7-AB' and r.get('timed') and r.get('attempt') in ('original', 'rerun')]
slots = {}
for r in recs:
    slots.setdefault((r['pass'], r['round'], r['row'], r['binary'], r['W']), {})[r['attempt']] = r


def ok(r):
    s, n = L.summary_of(open(os.path.join(r['cwd'], 'stdout.txt'), encoding='utf-8', errors='replace').read())
    c = L.load_csv(os.path.join(r['cwd'], 'run.csv'))
    good = (r.get('exit') == 0 and n == 1 and s['void_steps'] == 0 and s['pose_hash'] == L.POSE and s['expect_pose'] == 'match'
            and len(c['wall_ns']) == 500 and s['workers'] == r['W'] and not L.clean_why(r)
            and (r['binary'] != 's7p' or r['exe_sha256'] == TIP_SHA))
    return good, c


used = {}
for k, cand in slots.items():
    for att in ('original', 'rerun'):
        if att in cand:
            g, c = ok(cand[att])
            if g:
                used[k] = sum(c['wall_ns']) / 500 / 1e6
                break
P('S7-AB slots', len(slots), 'used', len(used))


def vals(row, b, W, ps=None):
    return [v for k, v in used.items() if k[2] == row and k[3] == b and k[4] == W and (ps is None or k[0] == ps)]


def comp(name, A, B):
    pooled = L.cmp_(L.cell(vals(*A)), L.cell(vals(*B)))
    passes = [L.cmp_(L.cell(vals(*A, ps)), L.cell(vals(*B, ps))) for ps in (0, 1, 2)]
    P('  %-22s n %d/%d B-A %+.4f (%+.2f %%) pooled %s passes %s -> %s' % (name, len(vals(*A)), len(vals(*B)), pooled['d'], pooled['pct'],
      L.yn(pooled), ' '.join(L.yn(c) for c in passes), L.ruling1(pooled, passes)))


P('== S7-AB ladder, s7p (= runner_tip_16191fda.exe), [0,500) wall, ruling 1')
P('  ref S7-JT s7p W8 median %.4f; W16 %.4f' % (statistics.median(vals('S7-JT', 's7p', 8)), statistics.median(vals('S7-JT', 's7p', 16))))
for W in (8, 16):
    for F in ('0.5', '1', '1.5', '2'):
        comp('ladder%d-F%s' % (W, F), ('S7-JT', 's7p', W), ('S7-ladder%d-F%s' % (W, F), 's7p', W))

procs = json.load(open(os.path.join(L.HERE, 'v_proc.json')))
P('')
P('== S4-AB armed W8 [100,500): per-wave TIP vs PARENT (ns), medians over K=9')
for b in ('parent', 'tip'):
    ps = [p for p in procs if p['block'] == 'S4-AB' and p['row'] == 'S4-JT-a' and p['bin'] == b and p['W'] == 8]
    f = lambda key, den: statistics.median(p['sums']['100..500'][key] / den(p) / p['tpn'] for p in ps)
    wv = statistics.median(p['sums']['100..500']['waves'] / 400 for p in ps)
    P('  %s: waves/step %.2f first ramp %.1f pass ramp %.1f join/wave %.1f' % (
        b, wv, f('phys_wave_first_ramp', lambda p: 400), f('phys_wave_pass_ramp', lambda p: 4800),
        f('phys_wave_join', lambda p: p['sums']['100..500']['waves'])))
open(os.path.join(L.HERE, 'v_posthoc.txt'), 'w').write(chr(10).join(OUT) + chr(10))
print(chr(10).join(OUT))
