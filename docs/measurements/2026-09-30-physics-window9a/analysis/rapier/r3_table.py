"""r3: the verdict table (markdown) from r1_claims.json; flags i/s/r pooled and per pass for each build; LETTER and
EXCL verdicts under both orientations (A = ours first, the pre-registered text; A = Rapier first, robustness)."""
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lib_rp as L  # noqa: E402

J = json.load(open(os.path.join(HERE, 'r1_claims.json')))
o = L.Out()


def c_(c, scale):
    return (f"{c['median'] / scale:.4f} [{c['min'] / scale:.4f}-{c['max'] / scale:.4f}] n{c['K']} "
            f"(IQR {100 * c['i']:.1f} %, SE {100 * c['s']:.1f} %)")


def short(v):
    v = v.replace('OURS FASTER CLAIMED vs every build', 'ours faster').replace('RAPIER FASTER CLAIMED vs ', 'Rapier faster vs ')
    v = v.replace('NOT CLAIMED (ours faster claimed vs rs4 only)', 'NOT CLAIMED (ours faster vs rs4 only)')
    return v


for win in ('[100,500)', '[0,100)', '[0,500)'):
    o(f'### window {win}')
    o('| claim | cells (ms, or ns/row for ROW) | Rapier/ours rs8, rs4 | i/s/r pooled; p0 p1 p2 (rs8 ; rs4) | LETTER | EXCL (ours-first / Rapier-first) |')
    o('|---|---|---|---|---|---|')
    for kind in ('WALL', 'ROW'):
        for cfg in 'DM':
            for W in (1, 2, 4, 8, 16):
                k = f'R-{kind}-{cfg}(W{W})|{win}'
                if k not in J['claims']:
                    continue
                d = J['claims'][k]
                scale = 1e6 if kind == 'WALL' else 1.0
                oc = d['rs8']['ours_cell']
                ks = '/'.join(str(a) for a, _ in d['rs8']['Ks'])
                cells = (f"ours {c_(oc, scale)} K/pass {ks}; rs8 {c_(d['rs8']['rap_cell'], scale)}; "
                         f"rs4 {c_(d['rs4']['rap_cell'], scale)}")
                rat = f"{d['rs8']['ratio_rapier_over_ours']:.3f}, {d['rs4']['ratio_rapier_over_ours']:.3f}"
                fl = ' ; '.join(f"{d[a]['pooled']}; {' '.join(d[a]['passes'])}" for a in ('rs8', 'rs4'))
                rb = d.get('robust_rapier_first', {})
                ex = short(d['EXCL'])
                exr = short(rb.get('EXCL', '?'))
                exs = ex if ex == exr else f'{ex} / {exr}'
                o(f"| R-{kind}-{cfg} W{W} | {cells} | {rat} | {fl} | {short(d['LETTER'])} | {exs} |")
    o()
o.save('r3_table.txt')
