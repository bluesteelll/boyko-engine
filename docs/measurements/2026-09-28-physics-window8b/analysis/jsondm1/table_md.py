"""Emits the verdict table of jsondm1.md (columns: claim id | pre-registered bar | cells + n | numbers | i/s/r |
verdict) from jsont.json and dm1.json. Writes table_md.txt. Flags are printed in the order i/s/r."""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
J = json.load(open(os.path.join(HERE, 'jsont.json')))
D = json.load(open(os.path.join(HERE, 'dm1.json')))
OUT = []


def isr(c):
    return ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n') + '/' + ('Y' if c['cl_r'] else 'n')


def num(c, d=4):
    return f"{c['median']:.{d}f} (IQR {c['iqr_abs']:.{d}f}, SE {c['se_abs']:.{d}f}, {c['min']:.{d}f}-{c['max']:.{d}f})"


def res(e):
    return max(max(c['bar_i'], c['bar_s']) for c in [e['pooled']] + e['passes_k2'])


BARS = {'0..100': 'ruling 8: "no measurable awake cost" -> ON not claimed slower (ruling 1 i AND s, every pass + pooled)',
        '100..500': 'ruling 8 product row: ON claimed faster (ruling 1; window 8 AllPairs -49 % W1 / -36 % W8)',
        '274..500': 'ruling 8 product row, frozen pile after step 274: ON claimed faster (ruling 1)'}
OUT.append('| claim id | pre-registered bar | cells + n | numbers (ms/step): OFF, ON = median (IQR, SE, min-max); ON/OFF | i/s/r pooled; p0 p1 p2 (smallest claimable) | verdict |')
OUT.append('|---|---|---|---|---|---|')
for win, cid in (('0..100', 'JST-AW'), ('100..500', 'JST-G'), ('274..500', 'JST-F')):
    for W in (1, 8):
        for arm in ('disarmed', 'armed'):
            e = J[f'{arm}|W{W}|{win}']
            rows = ('JSoffT', 'JSonT') if arm == 'disarmed' else ('JSoffT-a', 'JSonT-a')
            kp = '/'.join(str(c['KA']) for c in e['passes_k2'])
            kq = '/'.join(str(c['KB']) for c in e['passes_k2'])
            po = e['pooled']
            v, sg = e['verdict_k2'], e['sign_k2']
            if win == '0..100':
                verdict = 'NOT CLAIMED (no resolved awake cost)' if not v.startswith('CLAIMED') else (
                    'CLAIMED ON slower' if sg > 0 else 'CLAIMED ON faster')
            else:
                verdict = (v + ' (ON faster)') if v.startswith('CLAIMED') and sg < 0 else (
                    'REFUTED' if v.startswith('CLAIMED') else 'NOT CLAIMED')
            strict = e['verdict']
            if strict != v:
                verdict += f'; strict K>=3 guard: {strict} (p0 OFF K=2)'
            OUT.append(f"| {cid}-W{W}{'-a' if arm == 'armed' else ''} [{win.replace('..', ',')}) | {BARS[win]} | "
                       f"{rows[0]} n={e['off']['K']} ({kp}), {rows[1]} n={e['on']['K']} ({kq}) | OFF {num(e['off'])}; "
                       f"ON {num(e['on'])}; ON/OFF {po['ratio']:.4f} ({100 * (po['ratio'] - 1):+.2f} %, {po['delta']:+.4f} ms) | "
                       f"{isr(po)}; {' '.join(isr(c) for c in e['passes_k2'])} ({100 * res(e):.2f} %) | **{verdict}** |")
ZN = {'2': 'VB_SHADE', '4': 'VB_EARLY_CULL', '9': 'VB_RUN', '14': 'VB_PRODUCE_NET', '17': 'GBUF_DEFERRED_RESOLVE'}
BAR = ('ruling 9 / dm1 cut 11.5: paired median of B-A <= 0 or inside band max(abs(A1-A2), abs(B1-B2)) -> B<=A; above band = '
       'red for C6 (VB_EARLY_CULL above band -> drop C6)')
first = True
for k, v in D.items():
    row, z = k.split('|')
    if z not in ZN:
        continue
    short = row.replace('dm-', '').replace('-1920x1080', '')
    cid = ('DM1-EC-' if z == '4' else f'DM1-{ZN[z]}-') + short
    A, B, ph = v['A'], v['B'], v['ph']
    r2 = '[' + ' / '.join(v['R2']) + ']'
    red_any = any(x.startswith('RED') for x in [v['R1'], v['R3']])
    verdict = 'RED' if red_any else 'B <= A'
    if any(x.startswith('RED') for x in v['R2']):
        verdict += ' (R2 half RED, degenerate)'
    OUT.append(f"| {cid} | {BAR if first else 'as above'} | per-frame median over 220 frames; A n={A['K']}, B n={B['K']}, {v['npairs']} complete pairs | "
               f"A {num(A, 1)} us; B {num(B, 1)} us; d = median(B-A) {v['d']:+.2f} us ({v['pct']:+.2f} %); band {v['band']:.2f} us | "
               f"gate: R1 {v['R1']}; R2 {r2}; R3 {v['R3']}; all originals R1 {v['R1_all_orig']} "
               f"(d {v['d_all_orig']:+.2f} / band {v['band_all_orig']:.2f}); post hoc i/s/r {isr(ph)} | **{verdict}** |")
    first = False
open(os.path.join(HERE, 'table_md.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
print('\n'.join(OUT))
