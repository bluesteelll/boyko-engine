"""J-Son-T post-hoc reads (none of these is a pre-registered claim): manifold counts per window (per-manifold
normalisation check), W8 [0,100) per-process values with placement receipts, the L10 serial spans and the armed stage
spans on the frozen pile, and the per-step profile around the freeze. Writes jsont_post.txt."""
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import libjd as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


recs = L.load_recs('J-Son-T')
procs, used, dropped = L.select(recs, L.validate_runner)
WINS = {'0..100': (0, 100), '100..274': (100, 274), '274..500': (274, 500), '277..500': (277, 500), '100..500': (100, 500)}


def wm(p, col, a, b, scale=1.0):
    v = p['_c'].get(col)
    if v is None:
        return None
    x = L.mean(v[a:b])
    return None if x is None else x * scale


def sel(row, W):
    return [p for p in used if p['row'] == row and p['W'] == W]


P('# J-Son-T post hoc')
P('\n## manifold count per window (cell median of process means) - normalisation check')
for W in (1, 8):
    for row in ('JSoffT', 'JSonT'):
        P(f'W{W} {row}: ' + '; '.join(
            f'[{w}) {statistics.median([wm(p, "manifolds", a, b) for p in sel(row, W)]):.2f}' for w, (a, b) in WINS.items()))

P('\n## W8 [0,100) per process (ms) with the placement receipt (recorded, never used to drop)')
for row in ('JSoffT', 'JSonT', 'JSoffT-a', 'JSonT-a'):
    for p in sorted(sel(row, 8), key=lambda q: (q['pass'], q['round'])):
        pl = p.get('placement') or {}
        P(f'  {row:9s} p{p["pass"]} r{p["round"]} {p["attempt"]:8s} [0,100) {wm(p, "wall_ns", 0, 100, 1e-6):.4f} '
          f'[100,274) {wm(p, "wall_ns", 100, 274, 1e-6):.4f} [274,500) {wm(p, "wall_ns", 274, 500, 1e-6):.4f} '
          f'step0 {p["_c"]["wall_ns"][0] / 1e6:.3f} witness {p["others_busy_pct"]:.2f} top3 {pl.get("top3")} '
          f'top_share_of_proc {pl.get("top_share_of_proc")}')

P('\n## W1 placement receipts (main_share_top_est: exact at W1 on an idle machine), per row: min / median / max')
for row in ('JSoffT', 'JSonT', 'JSoffT-a', 'JSonT-a'):
    v = [(p.get('placement') or {}).get('main_share_top_est') for p in sel(row, 1)]
    v = [x for x in v if x is not None]
    P(f'  W1 {row}: n {len(v)} min {min(v):.3f} median {statistics.median(v):.3f} max {max(v):.3f}')
    for p in sel(row, 1):
        x = (p.get('placement') or {}).get('main_share_top_est')
        if x is not None and x < 0.5:
            P(f'     low-share process p{p["pass"]} r{p["round"]} {p["attempt"]}: share {x}, [0,100) '
              f'{wm(p, "wall_ns", 0, 100, 1e-6):.4f} ms')

P('\n## armed stage spans (us/step, cell median of process means): OFF JSoffT-a vs ON JSonT-a')
SPANS = [('bp', 'sys_physics_broadphase_colored_ns'), ('np', 'sys_physics_narrowphase_colored_ns'),
         ('graph', 'sys_physics_build_graph_ns'), ('solve', 'sys_physics_solve_colored_ns'),
         ('integrate', 'sys_physics_integrate_ns'), ('gather', 'sys_physics_gather_ns'),
         ('apply', 'sys_physics_apply_ns'), ('select_bp', 'sys_select_broadphase_ns'),
         ('sleep_begin', 'phys_sleep_begin_ns'), ('sleep_freeze', 'phys_sleep_freeze_ns'),
         ('sleep_end', 'phys_sleep_end_ns'), ('sleep_classify', 'phys_sleep_classify_ns'), ('g', 'g_ns'), ('u', 'u_ns'),
         ('r', 'r_ns'), ('sys_sum', 'sys_sum_ns')]
for W in (1, 8):
    for w in ('0..100', '100..274', '274..500', '277..500'):
        a, b = WINS[w]
        parts = []
        for name, col in SPANS:
            off = [wm(p, col, a, b, 1e-3) for p in sel('JSoffT-a', W)]
            on = [wm(p, col, a, b, 1e-3) for p in sel('JSonT-a', W)]
            off = [x for x in off if x is not None]
            on = [x for x in on if x is not None]
            mo = statistics.median(off) if off else float('nan')
            mn = statistics.median(on) if on else float('nan')
            parts.append(f'{name} {mo:.1f}->{mn:.1f}')
        wall_on = statistics.median([wm(p, 'wall_ns', a, b, 1e-3) for p in sel('JSonT-a', W)])
        wall_off = statistics.median([wm(p, 'wall_ns', a, b, 1e-3) for p in sel('JSoffT-a', W)])
        P(f'[{w}) W{W}: wall {wall_off:.1f}->{wall_on:.1f}; ' + '; '.join(parts))
    for w in ('274..500',):
        a, b = WINS[w]
        for name, col in (('bp_queried', 'phys_bp_queried'), ('bp_pairs', 'phys_bp_pairs'), ('np_pairs', 'phys_np_pairs'),
                          ('sleep_held', 'phys_sleep_held'), ('waves', 'waves'), ('colors', 'colors')):
            off = [wm(p, col, a, b) for p in sel('JSoffT-a', W)]
            on = [wm(p, col, a, b) for p in sel('JSonT-a', W)]
            off = [x for x in off if x is not None]
            on = [x for x in on if x is not None]
            P(f'   [{w}) W{W} {name}: OFF {statistics.median(off) if off else None} ON {statistics.median(on) if on else None}')

P('\n## per-step wall around the freeze, disarmed ON (median over the K processes, us); step = CSV row')
for W in (1, 8):
    ps = sel('JSonT', W)
    P(f'W{W}: ' + ', '.join(f'{s}: {statistics.median([p["_c"]["wall_ns"][s] for p in ps]) / 1e3:.0f}'
                            for s in list(range(268, 284)) + [300, 400, 499]))
    ps = sel('JSoffT', W)
    P(f'W{W} OFF: ' + ', '.join(f'{s}: {statistics.median([p["_c"]["wall_ns"][s] for p in ps]) / 1e3:.0f}'
                                for s in (0, 1, 2, 50, 99, 150, 273, 274, 300, 499)))
    ps = sel('JSonT', W)
    P(f'W{W} ON first steps: ' + ', '.join(f'{s}: {statistics.median([p["_c"]["wall_ns"][s] for p in ps]) / 1e3:.0f}'
                                          for s in (0, 1, 2, 50, 99)))
P('\n## stdout L10 lines of one ON process per W (verbatim)')
for W in (1, 8):
    p = sel('JSonT', W)[0]
    for line in L.read_text(os.path.join(p['cwd'], 'stdout.txt')).splitlines():
        if 'L10' in line or 'sleep' in line.lower():
            P(f'  W{W} p{p["pass"]} r{p["round"]}: {line[:400]}')
open(os.path.join(L.HERE, 'jsont_post.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')

P('\n## L10 serial pieces on JSonT-a: per-process sum of sleep_begin + freeze + end + classify (us/step), median; '
  'and as a share of the OFF (JSoffT-a) wall cell')
for W in (1, 8):
    for w in ('0..100', '100..274', '274..500'):
        a, b = WINS[w]
        sums = []
        for p in sel('JSonT-a', W):
            sums.append(sum(wm(p, c, a, b, 1e-3) for c in ('phys_sleep_begin_ns', 'phys_sleep_freeze_ns',
                                                             'phys_sleep_end_ns', 'phys_sleep_classify_ns')))
        offw = statistics.median([wm(p, 'wall_ns', a, b, 1e-3) for p in sel('JSoffT-a', W)])
        m = statistics.median(sums)
        P(f'  [{w}) W{W}: L10 serial sum {m:.2f} us/step = {100 * m / offw:.3f} % of the OFF wall {offw:.1f} us')
open(os.path.join(L.HERE, 'jsont_post.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
