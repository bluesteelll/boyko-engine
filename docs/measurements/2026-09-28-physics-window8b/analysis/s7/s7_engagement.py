"""S7 engagement receipt recomputed from the pre-flight's armed SUMMARY files (gate/s7/*-armed/stdout.txt) with the
window's own tools/s7pre8b.py (lanes_bound, engagement_why), plus a POST HOC DIAGNOSTIC of the same armed processes:
n = 1 per (exe, cfg, W), pre-flight (07:11, not under the quiet-window protocol), armed - mechanism hints only."""
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s7lib as L  # noqa: E402
import s7pre8b  # noqa: E402

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


S7 = os.path.join(L.GATE, 's7')
LABEL = {'P': 'uncapped', 'T': 'capped'}
got = {}
P(f'# Engagement receipt (physical = {s7pre8b.PHYSICAL}; bound = physical x color_scopes + (W+1) x setup_steps)')
for who in ('P', 'T'):
    for cfg in ('JT', 'JA'):
        for W in (8, 16):
            d = os.path.join(S7, f'{who}-{cfg}-W{W}-armed')
            s = L.D.parse_summary(open(os.path.join(d, 'stdout.txt'), 'rb').read())
            w = s['w8s']
            lanes, bound = s7pre8b.lanes_bound(w, W)
            why = s7pre8b.engagement_why(LABEL[who], W, w)
            got[(who, cfg, W)] = (s, w)
            P(f"  {who} {cfg} W{W}: pose {s['pose_hash']} {s['expect_pose']}, void {s['void_steps']}; lanes sum "
              f"{lanes:.0f} vs bound {bound} -> {'within' if lanes <= bound + 0.5 else 'ABOVE'}; color_scopes "
              f"{w['color_scopes']}, color_tasks {w['color_tasks']}, setup_steps {w['setup_steps']}, setup_tasks "
              f"{w['setup_tasks']}; receipt {'PASS' if not why else why}")
for cfg in ('JT', 'JA'):
    for W in (8, 16):
        a, b = got[('P', cfg, W)][1], got[('T', cfg, W)][1]
        eq = all(a[k] == b[k] for k in ('color_scopes', 'color_tasks', 'setup_steps', 'setup_tasks'))
        P(f'  {cfg} W{W}: P and T structural counts equal: {eq}')

P('\n# POST HOC DIAGNOSTIC (n = 1 each, pre-flight, armed; not a window measurement)')
P('  cfg W who | window mean ms | waves/step | lanes_mean | inflight_mean | ramp_ns | tail_ns | join_ns | imbalance_ns '
  '| np_lanes | np_tail_ns')
for cfg in ('JT', 'JA'):
    for W in (8, 16):
        for who in ('P', 'T'):
            s, w = got[(who, cfg, W)]
            P(f"  {cfg} {W} {who} | {s['window_mean_ns'] / 1e6:.4f} | {w['waves'] / w['steps']:.2f} | {w['lanes_mean']:.2f} | "
              f"{w['inflight_mean']:.2f} | {w['ramp_ns_mean']:.0f} | {w['tail_ns_mean']:.0f} | {w['join_ns_mean']:.0f} | "
              f"{w['imbalance_ns_mean']:.0f} | {w['np_lanes_mean']:.2f} | {w['np_tail_ns_mean']:.0f}")
P('\n# POST HOC arithmetic: the pooled T(16) - P(16) of the window per colour wave (colour scopes per step from the '
  'pre-flight SUMMARY; identical for P and T)')
import json  # noqa: E402
R = json.load(open(os.path.join(L.HERE, 's7_posthoc.json'), encoding='utf-8'))
C = json.load(open(os.path.join(L.HERE, 's7_results.json'), encoding='utf-8'))
for cfg, d_ms in (('JT', -C['claims']['0..500']['C3-JT']['gain_ms']),
                  ('JA', R['ratios']['0..500']['T16/P16 J-A']['delta'])):
    w = got[('T', cfg, 16)][1]
    per_step = w['color_scopes'] / w['steps']
    P(f'  {cfg}: T(16) - P(16) = {d_ms:+.4f} ms over {per_step:.2f} colour scopes/step = {1000 * d_ms / per_step:+.3f} us '
      f'per colour wave; tasks/colour wave {w["color_tasks"] / w["color_scopes"]:.2f}')
open(os.path.join(L.HERE, 's7_engagement.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
