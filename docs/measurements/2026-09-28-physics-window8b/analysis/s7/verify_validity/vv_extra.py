"""Side checks (post hoc, not pre-registered): processes > 1.5 % from their cell median with receipts; paired per
round T/P at each W (adjacent originals; the slot rule's used process); J-A P16 - T16; re-run distance from partner."""
import json
import os
import statistics
import collections

W = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b'
OUT = []


def say(s):
    OUT.append(s)
    print(s)


def m500(r):
    lines = open(os.path.join(r['cwd'], 'run.csv'), encoding='utf-8').read().splitlines()
    hdr = lines[0].split(',')
    i = hdr.index('wall_ns')
    v = [float(l.split(',')[i]) for l in lines[1:501]]
    return sum(v) / len(v) / 1e6


recs = [json.loads(l) for l in open(W + '/raw/runs.jsonl', encoding='utf-8') if l.strip()]
tim = [r for r in recs if r.get('block') == 'S7-AB' and 'row' in r and r.get('attempt') in ('original', 'rerun')]


def clean(r):
    return ((r['receipt_before']['cpu_avg'] <= 5) and (r['receipt_after']['cpu_avg'] <= 5)
            and (r.get('others_busy_pct') or 0) <= 2 and not r.get('build_proc_during'))


slots = collections.defaultdict(dict)
for r in tim:
    slots[(r['pass'], r['round'], r['row'], r['binary'], r['W'])][r['attempt']] = r
used = {}
for k, v in slots.items():
    o = v['original']
    used[k] = o if (o.get('valid') and clean(o)) else v.get('rerun')
cells = collections.defaultdict(list)
for k, r in used.items():
    cells[(k[2], k[3], k[4])].append((k, r, m500(r)))
say('processes > 1.5 % from their cell median, [0,500):')
n = 0
for c, lst in sorted(cells.items()):
    med = statistics.median(x for _, _, x in lst)
    for k, r, x in lst:
        dev = x / med - 1
        if abs(dev) > 0.015:
            n += 1
            say(f'  {c} p{k[0]} r{k[1]} {dev * 100:+.2f} % rb {r["receipt_before"]["cpu_avg"]} ra {r["receipt_after"]["cpu_avg"]} '
                f'witness {r.get("others_busy_pct")} attempt {r["attempt"]}')
say(f'count {n}')
say('paired per round T/P (used processes):')
for rid, w in (('S7-JT', 8), ('S7-JT', 16), ('S7-JA', 8), ('S7-JA', 16), ('S7-JT-W1', 1)):
    rat = []
    for p in (0, 1, 2):
        for rnd in (0, 1, 2):
            rat.append(m500(used[(p, rnd, rid, 's7t', w)]) / m500(used[(p, rnd, rid, 's7p', w)]))
    say(f'  {rid}@W{w}: median {statistics.median(rat):.4f} [{min(rat):.4f}-{max(rat):.4f}] T slower in {sum(x > 1 for x in rat)} of 9')
med = {c: statistics.median(x for _, _, x in lst) for c, lst in cells.items()}
say(f'J-A P16 - T16 = {med[("S7-JA", "s7p", 16)] - med[("S7-JA", "s7t", 16)]:+.4f} ms; T16/T8 JT {med[("S7-JT", "s7t", 16)] / med[("S7-JT", "s7t", 8)]:.4f}, '
    f'P16/P8 JT {med[("S7-JT", "s7p", 16)] / med[("S7-JT", "s7p", 8)]:.4f}, T16/T8 JA {med[("S7-JA", "s7t", 16)] / med[("S7-JA", "s7t", 8)]:.4f}, '
    f'P16/P8 JA {med[("S7-JA", "s7p", 16)] / med[("S7-JA", "s7p", 8)]:.4f}')
for k, v in slots.items():
    if 'rerun' in v:
        partner = slots[(k[0], k[1], k[2], 's7p' if k[3] == 's7t' else 's7t', k[4])]['original']
        say(f're-run {k}: original start {v["original"]["start"]}, re-run start {v["rerun"]["start"]}, partner start {partner["start"]}; '
            f're-run [0,500) {m500(v["rerun"]):.4f} vs original {m500(v["original"]):.4f} ms')
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'vv_extra_out.txt'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(OUT) + '\n')
