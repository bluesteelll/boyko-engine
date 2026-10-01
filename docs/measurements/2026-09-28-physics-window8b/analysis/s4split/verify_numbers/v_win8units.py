"""Post-hoc 1: were window 8 per-wave CSV counters TSC ticks? Reads win8/raw read-only. Output: v_win8units.txt."""
import json, os, statistics
import vlib as L

W8 = os.path.normpath(os.path.join(L.W8B, '..', 'win8'))
OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
recs = [json.loads(l) for l in open(os.path.join(W8, 'raw', 'runs.jsonl'), encoding='utf-8')]
sel = [r for r in recs if r.get('block') == 'W8S-A' and r.get('row') == 'J-T-a' and r.get('timed') and r.get('attempt') in ('original', 'rerun')]
P('win8 W8S-A J-T-a timed records', len(sel))
res = {}
W1WIDE = []
for W in (1, 8, 16):
    rows = []
    for r in sel:
        if r['W'] != W:
            continue
        s, n = L.summary_of(open(os.path.join(r['cwd'], 'stdout.txt'), encoding='utf-8', errors='replace').read())
        c = L.load_csv(os.path.join(r['cwd'], 'run.csv'))
        lo, hi = 100, 500
        sc = L.win_sum(c, 'phys_color_scopes', lo, hi)
        wv = L.win_sum(c, 'waves', lo, hi)
        if sc == 0:
            W1WIDE.append(L.win_mean(c, 'phys_color_wide_ns', lo, hi))
            continue
        tp = s.get('ticks_per_ns')
        w8 = s.get('w8s') or {}
        full_ratio = (L.win_sum(c, 'phys_wave_ramp', 0, 500) / L.win_sum(c, 'waves', 0, 500)) / w8['ramp_ns_mean'] if w8.get('ramp_ns_mean') else None
        wide = L.win_mean(c, 'phys_color_wide_ns', lo, hi)
        rows.append(dict(tp=tp, full_ratio=full_ratio,
                         ramp_raw=L.win_sum(c, 'phys_wave_ramp', lo, hi) / sc, tail_raw=L.win_sum(c, 'phys_wave_tail', lo, hi) / sc,
                         np_ramp_raw=L.win_mean(c, 'phys_np_wave_ramp', lo, hi), np_tail_raw=L.win_mean(c, 'phys_np_wave_tail', lo, hi),
                         wide=wide, waves=wv / 400.0, attempt=r['attempt'], p=r['pass'], rd=r['round']))
    res[W] = rows
    if not rows:
        continue
    m = lambda k: statistics.median(x[k] for x in rows)
    P('W%d n %d ticks_per_ns %.4f-%.4f; CSV/SUMMARY ramp ratio [0,500) %s' % (W, len(rows), min(x['tp'] for x in rows), max(x['tp'] for x in rows),
      sorted(set(round(x['full_ratio'], 4) for x in rows if x['full_ratio']))))
    P('   raw ramp/wave %.1f -> as-published us %.3f, in ns %.1f; raw tail/wave %.1f -> %.3f us published, ns %.1f' % (
        m('ramp_raw'), m('ramp_raw') / 1e3, statistics.median(x['ramp_raw'] / x['tp'] for x in rows), m('tail_raw'), m('tail_raw') / 1e3,
        statistics.median(x['tail_raw'] / x['tp'] for x in rows)))
    P('   np ramp raw %.1f (ns %.1f), np tail raw %.1f (ns %.1f)' % (m('np_ramp_raw'), statistics.median(x['np_ramp_raw'] / x['tp'] for x in rows),
      m('np_tail_raw'), statistics.median(x['np_tail_raw'] / x['tp'] for x in rows)))
if W1WIDE and res.get(8):
    P('W1 wide median over', len(W1WIDE), 'processes')
    wide1 = statistics.median(W1WIDE) / 1e6
    for W in (8, 16):
        rows = res[W]
        Lw = statistics.median(x['wide'] for x in rows) / 1e6 - wide1 / W
        wv = statistics.median(x['waves'] for x in rows)
        om = 1e3 * Lw / wv
        rt = statistics.median((x['ramp_raw'] + x['tail_raw']) / x['tp'] for x in rows) / 1e3
        P('W%d omega %.3f us; ramp+tail in ns-units %.3f us = %.1f %% of omega' % (W, om, rt, 100 * rt / om))
    g8 = statistics.median(x['tail_raw'] / x['tp'] for x in res[8]); g16 = statistics.median(x['tail_raw'] / x['tp'] for x in res[16])
    P('W16-W8 tail growth: %.3f us/wave x 96 = %.4f ms (published: +1.66 us/wave, ~0.16 ms)' % ((g16 - g8) / 1e3, 96 * (g16 - g8) / 1e6))
open(os.path.join(L.HERE, 'v_win8units.txt'), 'w').write(chr(10).join(OUT) + chr(10))
print(chr(10).join(OUT))
