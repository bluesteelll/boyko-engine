"""v4: what a 2 % witness means in CPU time for a C4-AB process (others_busy_pct = others_cpu_s / (wall_s * logical CPUs))."""
import statistics
import vlib as L

o = L.Out()
recs = [r for r in L.records() if r.get('block') == 'C4-AB' and 'row' in r and r.get('attempt') in ('original', 'rerun')]
chk = [abs(r['others_cpu_s'] / (r['wall_s'] * 16) * 100 - r['others_busy_pct']) for r in recs if r.get('wall_s')]
o(f'others_busy_pct == others_cpu_s/(wall_s*16)*100 within {max(chk):.3f} pp over {len(chk)} processes')
for kind in ('runner', 'jolt'):
    ws = [r['wall_s'] for r in recs if r['kind'] == kind]
    m = statistics.median(ws)
    o(f'{kind}: median process wall {m:.3f} s -> 2 % witness = {0.02 * 16 * m * 1000:.0f} ms of other-process CPU '
      f'(= {0.02 * 16 * m * 100:.0f} % of one logical CPU over the run)')
o.save('v4_witness.txt')
