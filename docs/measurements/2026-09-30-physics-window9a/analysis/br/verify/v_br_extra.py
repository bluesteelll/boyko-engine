"""Extra receipts for the C4-BR verification: clock witness (median AND mean), startup scene set per exe,
criterion base==new, and the static placement re-run (v_place.json, produced by tree-c4/m9/place.py)."""
import filecmp, json, os, re, statistics
W9A = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a'
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = []
def P(s=''):
    OUT.append(str(s)); print(s)
recs = [json.loads(l) for l in open(W9A + '/raw/runs.jsonl', encoding='utf-8') if l.strip()]
br = [r for r in recs if r.get('block') == 'C4-BR' and r.get('run_tag') == '033740' and 'row' in r]
P('== clock witness (PDH _Total, 1-s samples over the process life)')
for r in br:
    p = [x['total'] for x in r['perf']]
    P('seq %d %-6s n %d median %.2f mean %.2f min %.1f; samples < 120 %%: %d (%.0f %%)' % (
        r['seq'], r['binary'], len(p), statistics.median(p), statistics.fmean(p), min(p), sum(1 for x in p if x < 120), 100 * sum(1 for x in p if x < 120) / len(p)))
P('\n== startup scene set (stderr kernel receipts) and criterion base==new')
for r in br:
    d = r['cwd'].replace(chr(92), '/')
    se = open(d + '/stderr.txt', encoding='utf-8', errors='replace').read()
    sizes = sorted(set(int(m) for m in re.findall(r'^bp_g4_uniform/(\d+):', se, re.M)))
    kd = len(re.findall(r'kernel LeafListKd', se))
    same = all(filecmp.cmp(os.path.join(root, 'sample.json'), os.path.join(os.path.dirname(root), 'base', 'sample.json'), shallow=False)
               for root, _, files in os.walk(d + '/criterion') if os.path.basename(root) == 'new')
    P('seq %d %-6s uniform sizes %d (max %d), LeafListKd receipts %d, base==new %s' % (r['seq'], r['binary'], len(sizes), max(sizes), kd, same))
P('\n== static placement of all_pairs_into (tree-c4/m9/place.py apinto_16191fda.dis e0 192 <exe>)')
pl = json.load(open(HERE + '/v_place.json'))
for k, v in pl['exes'].items():
    P('%s: hits %s loop head mod64 %s, 64-B lines %s, jcc on/across 32 B %d' % (
        os.path.basename(k), v['hits'], v.get('loop_head_rva_mod64'), v.get('loop_64B_lines'),
        sum(1 for j in v.get('jcc_in_loop', []) if j['crosses32'] or j['ends_on32'])))
m9 = json.load(open(W9A.replace('win9a', 'tree-c4') + '/m9/place_apinto.json'))
P('M9 record place_apinto.json (incl. M9 rebuilds without the G4-sizes patch):')
for k, v in m9['exes'].items():
    P('  %s: hits %s loop head mod64 %s, 64-B lines %s, jcc on/across 32 B %d' % (
        k.replace(chr(92), '/').split('/')[-2] + '/' + os.path.basename(k), v['hits'], v.get('loop_head_rva_mod64'), v.get('loop_64B_lines'),
        sum(1 for j in v.get('jcc_in_loop', []) if j['crosses32'] or j['ends_on32'])))
open(HERE + '/v_br_extra.txt', 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
