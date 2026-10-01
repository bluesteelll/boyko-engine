"""o6: who made the omega blocks' processes dirty: the top processes of the after-receipt and of the during-process
witness, for every process that failed window 8's clean rule (the reason the slot was re-run or dropped)."""
import collections
import json
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
recs = {(r['block'], r['pass'], r['round'], r['row'], r.get('attempt')): r for r in L.load_recs()}
cnt_after, cnt_wit = collections.Counter(), collections.Counter()
O('# o6 dirty processes (window 8 clean rule) in omega-v2 / omega-v1-cont: reason | receipt-after top 2 | witness top 2')
n = 0
for k, r in sorted(recs.items(), key=lambda kv: kv[1].get('start') or ''):
    why = L.clean_why(r)
    if not why or r.get('attempt') == 'warmup':
        continue
    n += 1
    ra = [(o['name'], o['cpu_s']) for o in ((r.get('receipt_after') or {}).get('top5') or [])[:2]]
    wt = [(o['name'], o['cpu_s']) for o in (r.get('others_top5') or [])[:2]]
    if ra:
        cnt_after[ra[0][0]] += 1
    if wt:
        cnt_wit[wt[0][0]] += 1
    O('  %s p%d r%d %-20s %-8s %s | after %s | witness %s' % (k[0], k[1], k[2], k[3], k[4], why, ra, wt))
O('dirty timed processes: %d; top after-receipt process counts %s; top witness process counts %s' % (
    n, dict(cnt_after), dict(cnt_wit)))
O.save('o6.txt')
