"""Small receipts/placement checks for the section's validity notes. Writes checks.txt."""
import collections
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
tops = collections.Counter((p['W'], (p.get('placement') or {}).get('top_cpu')) for p in used)
P(f'J-Son-T used processes by (W, top CPU): {dict(tops)}')
cov = collections.Counter((p['W'], (p.get('placement') or {}).get('main_share_top_est') is not None) for p in used)
P(f'J-Son-T used processes carrying main_share_top_est by W: {dict(cov)}')
fr = [(p.get('placement') or {}).get('main_cycle_frac') for p in used if p['W'] == 1]
P(f'J-Son-T W1 main_cycle_frac: min {min(fr)} max {max(fr)}')
P(f'J-Son-T ON rows first_frozen_step over ALL records: {sorted(set(p["_s"]["first_frozen_step"] for p in procs if p["row"].startswith("JSon")))}')
bt = sorted(set(str({k: p['_s']['broadphase_tree'][k] for k in ('static_rebuilds', 'sleeper_rebuilds', 'members', 'evictions')}) + ' ' + p['row'] for p in procs))
P('TreeDiag (key fields) by row: ' + '; '.join(bt))
P('pass start/end times: ' + '; '.join(f'p{ps} {min(p["start"] for p in procs if p["pass"] == ps)[11:19]}-{max(p["end"] for p in procs if p["pass"] == ps)[11:19]}' for ps in (0, 1, 2)))
dm = L.load_recs('DM1')
dprocs, dused, ddropped = L.select(dm, L.validate_dm1, dm1=True)
P(f'DM1 used: receipts before max {max(p["receipt_before"]["cpu_avg"] for p in dused)}, after max '
  f'{max(p["receipt_after"]["cpu_avg"] for p in dused)}, witness max {max(p["others_busy_pct"] for p in dused)}')
unc = [(p['seq'], p['attempt'], p['clean_why'], [o['name'] for o in (p.get('others_top5') or [])[:1]]) for p in dprocs if p['clean_why']]
P(f'DM1 unclean processes ({len(unc)}): {unc}')
P(f'DM1 time span: {min(p["start"] for p in dprocs)[11:19]}-{max(p["end"] for p in dprocs)[11:19]}')
ws = [p for p in procs + dprocs if p['clean_why']]
top = collections.Counter(((p.get('others_top5') or [{}])[0]).get('name') for p in ws)
P(f'top witness process among the unclean J-Son-T + DM1 processes: {dict(top)}')
open(os.path.join(L.HERE, 'checks.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')

import json  # noqa: E402
D = json.load(open(os.path.join(L.HERE, 'dm1.json')))
for k, v in D.items():
    P(f"DM1 {k}: band {v['band']:.2f} us = {100 * v['band'] / v['A']['median']:.1f} % of the A cell median {v['A']['median']:.1f}; "
      f"pair diffs {[round(x, 2) for x in v['diffs']]}")
J = json.load(open(os.path.join(L.HERE, 'jsont.json')))
for W in (1, 8):
    e = J[f'disarmed|W{W}|274..500']
    P(f"J-Son-T W{W} [274,500): ON floor = {100 * e['on']['median'] / e['off']['median']:.2f} % of the OFF step")
man_on, man_off = 4467.00, 4468.11
P(f'manifold ratio OFF/ON on [274,500) (cells from jsont_post.txt): {man_off / man_on:.5f}')
open(os.path.join(L.HERE, 'checks.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
