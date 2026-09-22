"""Assembles window_report.md from head.txt + raw/tables.md + raw/sensitivity.md + tail.txt (the task's deliverable)."""
import os
SPW = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
parts = []
for name in ('tools/head.txt', 'raw/tables.md', 'tools/mid.txt', 'raw/sensitivity.md', 'tools/tail.txt'):
    p = os.path.join(SPW, name)
    if os.path.isfile(p):
        parts.append(open(p, encoding='utf-8').read().rstrip() + '\n')
    else:
        parts.append(f'(missing {name})\n')
out = os.path.join(SPW, 'window_report.md')
open(out, 'w', encoding='utf-8').write('\n'.join(parts))
print('wrote', out, sum(len(p) for p in parts), 'chars')
