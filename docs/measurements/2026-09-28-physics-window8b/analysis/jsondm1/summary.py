"""Builds the verdict-table rows of jsondm1.md from jsont.json and dm1.json (outputs of jsont.py and dm1.py, run
first). Writes summary.txt (markdown rows + the resolution of each J-Son-T comparison)."""
import json
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
J = json.load(open(os.path.join(HERE, 'jsont.json')))
D = json.load(open(os.path.join(HERE, 'dm1.json')))
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def flags(c):
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def cellstr(c):
    return (f"{c['median']:.4f} (IQR {c['iqr_abs']:.4f}, SE {c['se_abs']:.4f}, {c['min']:.4f}-{c['max']:.4f})")


def resolution(e):
    allc = [e['pooled']] + e['passes_k2']
    return max(max(c['bar_i'], c['bar_s']) for c in allc)


def label(e, gain):
    v, sign = e['verdict_k2'], e['sign_k2']
    if not v.startswith('CLAIMED'):
        return 'NOT CLAIMED'
    if gain:
        return v if sign < 0 else 'REFUTED (' + v + ' ON slower)'
    return ('CLAIMED (ON slower) ' + v[7:]).strip() if sign > 0 else 'REFUTED (ON faster)'


P('| claim id | pre-registered bar | cells + n (K per pass) | OFF ms/step: median (IQR, SE, min-max) | ON ms/step: median (IQR, SE, min-max) | ON/OFF pooled | r/i/s pooled; per pass p0 p1 p2 | verdict (strict K>=3 guard) | smallest claimable |')
P('|---|---|---|---|---|---|---|---|---|')
CL = [('JST-AW', '0..100', False, 'awake cost: ON not claimed slower (ruling 8 "no measurable awake cost"; ruling 1 rule)'),
      ('JST-G', '100..500', True, 'ON claimed faster under ruling 1 (window 8: -49 % W1 / -36 % W8 on AllPairs)'),
      ('JST-F', '274..500', True, 'ON claimed faster under ruling 1 (settled pile after the step-274 freeze)')]
for cid, win, gain, bar in CL:
    for W in (1, 8):
        for arm in ('disarmed', 'armed'):
            e = J[f'{arm}|W{W}|{win}']
            rows = ('JSoffT', 'JSonT') if arm == 'disarmed' else ('JSoffT-a', 'JSonT-a')
            kp = [f"{c['KA']}/{c['KB']}" for c in e['passes_k2']]
            pp = ' '.join(flags(c) for c in e['passes_k2'])
            strict = e['verdict'] + ('' if e['sign'] is None else (' ON faster' if e['sign'] < 0 else ' ON slower'))
            P(f"| {cid}-W{W}{'-a' if arm == 'armed' else ''} [{win.replace('..', ',')}) | {bar} | "
              f"{rows[0]} n={e['off']['K']}, {rows[1]} n={e['on']['K']} (per pass OFF/ON {', '.join(kp)}) | "
              f"{cellstr(e['off'])} | {cellstr(e['on'])} | {e['pooled']['ratio']:.4f} ({100 * (e['pooled']['ratio'] - 1):+.2f} %, "
              f"{e['pooled']['delta']:+.4f} ms) | {flags(e['pooled'])}; {pp} | **{label(e, gain)}** ({strict}) | "
              f"{100 * resolution(e):.2f} % |")
P('')
P('post hoc windows (not pre-registered):')
for win in ('0..500', '100..274', '277..500'):
    for W in (1, 8):
        for arm in ('disarmed', 'armed'):
            e = J[f'{arm}|W{W}|{win}']
            P(f"  [{win}) W{W} {arm}: OFF {e['off']['median']:.4f} (n {e['off']['K']}) ON {e['on']['median']:.4f} (n {e['on']['K']}) "
              f"ratio {e['pooled']['ratio']:.4f} ({100 * (e['pooled']['ratio'] - 1):+.2f} %) pooled {flags(e['pooled'])}; "
              f"passes {' '.join(flags(c) for c in e['passes_k2'])}; {e['verdict_k2']}; smallest claimable {100 * resolution(e):.2f} %")
P('')
ZN = {'2': 'VB_SHADE', '4': 'VB_EARLY_CULL', '9': 'VB_RUN', '14': 'VB_PRODUCE_NET', '17': 'GBUF_DEFERRED_RESOLVE',
      '5': 'VB_EARLY_RASTER', '12': 'VB_PRODUCE_RUN'}
P('| claim id | pre-registered bar | row, zone | d = median(B-A), us (% of mean A) | band, us (R1) | R1 | R2 halves | R3 (band) | all originals R1 (d / band) | verdict |')
P('|---|---|---|---|---|---|---|---|---|---|')
for k, v in D.items():
    row, z = k.split('|')
    if z in ('5', '12'):
        continue
    bar = 'B <= A: paired median of B-A <= 0 or inside max(|A1-A2|,|B1-B2|) (cut 11.5; ruling 9)'
    red = any(x.startswith('RED') for x in [v['R1'], v['R3']])
    verdict = 'RED (C6 fails)' if red else ('B <= A' + (' (one degenerate R2 half RED)' if any(x.startswith('RED') for x in v['R2']) else ''))
    cid = 'DM1-' + ('EC' if z == '4' else 'Z' + z) + '-' + row.replace('dm-', '').replace('-1920x1080', '')
    P(f"| {cid} | {bar} | {row}, {ZN[z]} ({z}) | {v['d']:+.2f} ({v['pct']:+.2f} %) | {v['band']:.2f} | {v['R1']} | "
      f"{'; '.join(v['R2'])} | {v['R3']} ({v['band3']:.2f}) | {v['R1_all_orig']} ({v['d_all_orig']:+.2f} / {v['band_all_orig']:.2f}) | **{verdict}** |")
open(os.path.join(HERE, 'summary.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
