"""Verifier extras: stricter V4/V5/V6 checks against pins.json, the J-T/J-D A/A, the p0 slow Rapier W1 processes,
and the unclean attribution of C4-AB p0. Reuses v1_recompute by exec (same directory, the verifier's own code)."""
import io, contextlib, json, os, statistics
HERE = os.path.dirname(os.path.abspath(__file__))
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    G = {'__file__': os.path.join(HERE, 'v1_recompute.py'), '__name__': 'v1'}
    exec(open(os.path.join(HERE, 'v1_recompute.py'), encoding='utf-8').read(), G)
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a)
    OUT.append(s)
    print(s)

PINS = G['PINS']; sel = G['sel']; info = G['info']; ROWS = G['ROWS']; BINS = G['BINS']
bad = []
for r in sel:
    if r['kind'] != 'rapier':
        continue
    txt = open(os.path.join(r['cwd'], 'stdout.txt'), encoding='utf-8', errors='replace').read()
    s = json.loads([l for l in txt.splitlines() if l.startswith('SUMMARY')][0][len('SUMMARY'):].strip())
    arm = BINS[r['binary']]['arm']
    cfgname = ROWS[r['row']]['args'][1]
    pin = PINS['arms'][arm]['cfgs'][cfgname]
    why = []
    if s.get('config') != pin['config_timed']:
        why.append('V5 config != config_timed')
    if s.get('source_fnv1a64') != PINS['source_fnv1a64']:
        why.append('V4 source hashes')
    tf = s.get('target_features')
    if not (isinstance(tf, dict) and all(tf.get(k) is True for k in ('avx2', 'fma', 'bmi2'))):
        why.append('V4 target_features %s' % tf)
    if s.get('simd_lanes') != PINS['arms'][arm]['lanes']:
        why.append('V4 lanes')
    if s.get('pose_hash') != pin['pose_hash']:
        why.append('V7 pose_hash')
    if s.get('target_env') != 'msvc':
        why.append('target_env')
    if not pin.get('twin_ok'):
        why.append('V9 twin not ok')
    sc = s.get('scene')
    blob = json.dumps(s)
    if PINS['spawn_hash'] not in blob:
        why.append('V6 spawn hash absent')
    if why:
        bad.append((r['row'], r['binary'], r['W'], r['pass'], why))
P('stricter Rapier checks (V4 sources/features/lanes, V5 config == pins config_timed, V6 spawn hash present, V7 pose_hash, V9 twin_ok):',
  '%d of %d processes fail' % (len(bad), sum(1 for r in sel if r['kind'] == 'rapier')))
for b in bad[:8]:
    P('  ', b)

series = G['series']; cell = G['cell']; judge = G['judge']; fl = G['fl']; lab = G['lab']
P('')
P('== post hoc A/A: J-T#tip (A) vs J-D#tip (B), ruling-1 flags (EXCL = skip K<3 passes)')
for win in (1, 0):
    for W in (1, 8, 16):
        j = judge(series('C4-AB', 'C4-JT', 'tip', W, win), series('C4-AB', 'C4-JD', 'tip', W, win), 'EXCL')
        P('  %s W%d: J-T %.4f J-D %.4f B/A %.4f (%+.2f %%) %s -> EXCL %s' % (G['WIN'][win], W, j['A']['m'], j['B']['m'], j['pooled']['ratio'], 100 * (j['pooled']['ratio'] - 1), fl(j), lab(j)))

P('')
P('== RP-D rs8 W1, [100,500), every used process by pass/round (ms; receipts before/after/witness)')
for r in sorted([r for r in G['used'] if r['block'] == 'C4-RAPIER' and r['row'] == 'RP-D' and r['binary'] == 'rs8' and r['W'] == 1], key=lambda r: (r['pass'], r['round'])):
    P('  p%d r%d %s: %.4f  (%.2f / %.2f / %.2f)' % (r['pass'], r['round'], r['attempt'], info[id(r)][2][1], r['receipt_before']['cpu_avg'], r['receipt_after']['cpu_avg'], r['others_busy_pct']))

P('')
P('== C4-AB p0 unclean attribution (J-T/J-D/jolt56 rows): top other process of the witness, and of the after-receipt when that flagged')
from collections import Counter
cw = Counter(); ca = Counter(); cany = Counter()
for r in sel:
    if r['block'] != 'C4-AB' or r['pass'] != 0 or not info[id(r)][1]:
        continue
    names = set()
    if 'witness' in info[id(r)][1]:
        n = (r.get('others_top5') or [{}])[0].get('name'); cw[n] += 1; names.add(n)
    if 'after' in info[id(r)][1]:
        n = (r['receipt_after'].get('top5') or [{}])[0].get('name'); ca[n] += 1; names.add(n)
    for n in names:
        cany[n] += 1
P('  witness top:', dict(cw)); P('  after-receipt top:', dict(ca)); P('  union per process:', dict(cany))
open(os.path.join(HERE, 'v2_extras.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
