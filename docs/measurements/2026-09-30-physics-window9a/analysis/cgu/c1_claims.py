"""C4-CGU claims under ruling 1 (lib9a.judge, LETTER reading; every pass-cell here has K = 3, so LETTER = GATING-ONLY).
A = tip (cgu 16), B = tipcgu1 (cgu 1); B/A < 1 => cgu1 faster. PRIMARY window [100,500); [0,500) beside.
Pre-registered: "cgu1 claimed faster than cgu16 at W8 AND not claimed slower at any W -> the shipped/parity
profile moves to cgu 1"."""
import json

import cgu_lib as C

L = C.L
o = C.Out()
recs = C.all_records()
procs, used, dropped = C.select(C.BLOCK, recs, C.RUN_CGU, row=C.ROW)
o(f'# C4-CGU claims; used {len(used)}, dropped {len(dropped)}')
res = {}


def verdict(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    who = 'cgu1 faster' if j['dir'] == 'B<A' else 'cgu1 SLOWER'
    return f"{who} {'CLAIMED STRONG' if j['strong'] else 'CLAIMED'}"


for win in (C.PRIMARY, C.BESIDE):
    o(f'\n## window {win} ' + ('(PRIMARY)' if win == C.PRIMARY else '(beside, decides nothing)'))
    for W in C.WS:
        va = C.vals(used, C.A_KEY, W, win)
        vb = C.vals(used, C.B_KEY, W, win)
        j = L.judge(va, vb, 'LETTER')
        p = j['pooled']
        o(f'W{W}: A cgu16 {L.fcell(j["a"])}')
        o(f'     B cgu1  {L.fcell(j["b"])}')
        o(f'     {L.fj(j)}')
        for k in L.PASSES:
            ca, cb = L.cell(va.get(k, [])), L.cell(vb.get(k, []))
            c = j['per'][k]
            o(f'     p{k}: A {ca["median"]:.4f} [{ca["min"]:.4f}-{ca["max"]:.4f}] B {cb["median"]:.4f} [{cb["min"]:.4f}-{cb["max"]:.4f}] '
              f'B/A {c["ratio"]:.4f} ({100*(c["ratio"]-1):+.2f} %) bars i {100*c["bar_i"]:.2f} s {100*c["bar_s"]:.2f} r {100*c["bar_r"]:.2f} % -> {L.yn(c)}{" sep" if c["minmax_sep"] else ""}')
        o(f'     VERDICT {verdict(j)}; magnitude {100*(p["ratio"]-1):+.2f} % ({p["delta"]*1000:+.1f} us); '
          f'pooled resolution (larger of the i/s bars) {100*max(p["bar_i"], p["bar_s"]):.2f} %')
        res[f'{win}|W{W}'] = {
            'A': j['a'], 'B': j['b'], 'ratio': p['ratio'], 'delta_ms': p['delta'], 'pooled_flags': L.yn(p),
            'per_pass_flags': {k: L.yn(c) for k, c in j['per'].items()},
            'per_pass_ratio': {k: c['ratio'] for k, c in j['per'].items()},
            'bars_pooled': {'i': p['bar_i'], 's': p['bar_s'], 'r': p['bar_r']}, 'sep_pooled': p['minmax_sep'],
            'claimed': j['claimed'], 'strong': j['strong'], 'dir': j['dir'], 'verdict': verdict(j)}

o('\n## The pre-registered claim (PRIMARY window [100,500))')
w8 = res[f'{C.PRIMARY}|W8']
faster8 = w8['claimed'] and w8['dir'] == 'B<A'
slower_any = [W for W in C.WS if res[f'{C.PRIMARY}|W{W}']['claimed'] and res[f'{C.PRIMARY}|W{W}']['dir'] == 'B>A']
o(f'cgu1 claimed faster at W8: {faster8} ({w8["verdict"]}; B/A {w8["ratio"]:.4f}, pooled {w8["pooled_flags"]}, passes {w8["per_pass_flags"]})')
o(f'cgu1 claimed slower at any W: {slower_any or "none"}')
claim = faster8 and not slower_any
o(f'CLAIM: {"CLAIMED" if claim else "NOT CLAIMED"} -> PROFILE MOVES TO cgu 1: {"YES" if claim else "NO"}')
o('\n## Same rule on [0,500) (beside, decides nothing)')
w8b = res[f'{C.BESIDE}|W8']
slower_b = [W for W in C.WS if res[f'{C.BESIDE}|W{W}']['claimed'] and res[f'{C.BESIDE}|W{W}']['dir'] == 'B>A']
o(f'W8: {w8b["verdict"]} (B/A {w8b["ratio"]:.4f}); slower anywhere: {slower_b or "none"}; '
  f'rule would read {"CLAIMED" if (w8b["claimed"] and w8b["dir"] == "B<A" and not slower_b) else "NOT CLAIMED"}')
o('\n## Magnitude table (B/A - 1, %, pooled medians)')
o('W  | [100,500) | [0,500)')
for W in C.WS:
    o(f'{W:<2d} | {100*(res[f"{C.PRIMARY}|W{W}"]["ratio"]-1):+.2f} % ({res[f"{C.PRIMARY}|W{W}"]["verdict"]}) | '
      f'{100*(res[f"{C.BESIDE}|W{W}"]["ratio"]-1):+.2f} % ({res[f"{C.BESIDE}|W{W}"]["verdict"]})')
json.dump(res, open(C.os.path.join(C.HERE, 'c1_claims.json'), 'w'), indent=1, default=str)
o.save('c1_claims.txt')
