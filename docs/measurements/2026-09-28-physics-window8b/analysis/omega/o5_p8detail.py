"""o5: the headline cells (P8, b = 4P, gap 0) process by process: slope, region@36/@72, first helper, parks, with the
process's placement receipt beside it (recorded only; nothing is dropped on it). Also the P8 b=1P/2P spread."""
import json
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
PER = json.load(open(L.HERE + '/o1_per_process.json'))
procs = {(p['row'], p['pass'], p['round'], p['attempt']): p for p in L.all_procs()}
O('# o5 P8 per-process detail (ns); placement = top CPU busy %, main-thread cycle share (recorded only)')
for route in ('worker', 'external'):
    for bpp in (1, 2, 4):
        for helper in ('spin', 'park'):
            xs = [x for x in PER if tuple(x['key']) == (route, 8, bpp, helper, 0)]
            O('\n## %s P8 b=%dP %s' % (route, bpp, helper))
            for x in sorted(xs, key=lambda x: (x['pass'], x['round'])):
                pl = procs[(x['row'], x['pass'], x['round'], x['attempt'])]['placement']
                O('  p%d r%d %-8s slope %6.0f  r36 %7d r72 %7d  first36/72 %5s/%5s  parks36/72 %s/%s  allact72 %4d  top %s main_frac %s' % (
                    x['pass'], x['round'], x['attempt'], x['slope'], x['r36'], x['r72'], x['first36'], x['first72'],
                    x['parks36'], x['parks72'], x['allact72'], (pl.get('top3') or [[None, None]])[0], pl.get('main_cycle_frac')))
O.save('o5.txt')
