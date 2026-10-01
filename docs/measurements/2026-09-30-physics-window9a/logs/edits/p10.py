import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
apply('tools/gate9a.py', [
 ("env=env, timeout=300)", "env=env, timeout=1800)"),
 ("""def part_list():
    out = []
    for rid, row in ROWS.items():
        if row['kind'] != 'criterion':
            continue
""", """def part_list(only=None):
    out = []
    for rid, row in ROWS.items():
        if row['kind'] != 'criterion' or (only and rid not in only):
            continue
"""),
 ("""            log(f'list {rid:10s} {key:6s} exit {rc} listed {len(ids)} expected {len(need)} missing {miss[:3]} unexpected {extra[:3]} '
                f'{"PASS" if ok else "FAIL"}')
    json.dump(out, open(os.path.join(GATE, 'gate_list.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
""", """            log(f'list {rid:10s} {key:6s} exit {rc} listed {len(ids)} expected {len(need)} missing {miss[:3]} unexpected {extra[:3]} '
                f'{"PASS" if ok else "FAIL"}')
            p = os.path.join(GATE, 'gate_list.json')     # written after every process: a timeout later does not lose the earlier rows
            old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) else []
            keep = [r for r in old if (r['row'], r['binary']) != (rid, key)]
            json.dump(keep + [out[-1]], open(p, 'w', encoding='utf-8', newline='\n'), indent=1)
"""),
 ("""    else:
        {'fixtures': part_fixtures, 'preflight': part_preflight, 'red': part_red, 'r4': part_r4, 'list': part_list}[part]()""",
  """    elif part == 'list':
        part_list(only)
    else:
        {'fixtures': part_fixtures, 'preflight': part_preflight, 'red': part_red, 'r4': part_r4}[part]()"""),
])
