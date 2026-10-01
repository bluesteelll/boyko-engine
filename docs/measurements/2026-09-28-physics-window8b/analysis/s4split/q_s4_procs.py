"""Per-process listing (diagnostic) for the S4-AB cells that decide the verdicts: wall [0,500), step-time spread,
placement receipt, witness. Nothing here is used to drop a process."""
from common_s4 import *

B = 'S4-AB'


def burst(p):
    c = L.load_csv(os.path.join(p['cwd'], 'run.csv'))
    w = [x for x in c['wall_ns'][0:500]]
    med = statistics.median(w)
    return med / 1e6, sum(1 for x in w if x > 1.2 * med) / len(w)


for row, W, bins in (('S4-JT', 16, ('parent', 'tip')), ('S4-JT-a', 16, ('parent', 'tip')), ('S4-JT', 8, ('parent', 'tip')),
                     ('S4-rung', 8, ('tip',)), ('S4-JT', 1, ('parent', 'tip')), ('S4-JT', 2, ('parent', 'tip'))):
    P('\n## %s W%d' % (row, W))
    for b in bins:
        for p in sorted(sel(row, W, B, b), key=lambda q: (q['pass'], q['round'])):
            pl = p.get('placement') or {}
            med, bf = burst(p)
            P('%s p%d r%d %s: mean %.4f median-step %.4f ms, frames>1.2xmed %.1f %%, span %s, start %s, witness %.2f %%, '
              'rcpt %.2f/%.2f, top3 %s, main_share_top %.3f, main_cycle_frac %s' % (
                  b, p['pass'], p['round'], p['attempt'], p['wall_ms']['0..500'], med, 100 * bf,
                  '%.4f' % (p['stats']['0..500']['phys_solve_build_ns'] / 1e6) if row.endswith('-a') else '-',
                  p['start'][11:19], p['others_busy_pct'], p['receipt_before'], p['receipt_after'], pl.get('top3'),
                  pl.get('main_share_top_est') or 0, pl.get('main_cycle_frac')))
save('q_s4_procs.txt')
