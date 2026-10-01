src = open('D:/tmp/rapier-parity/gate/analyze.py', encoding='utf-8').read()
i = src.index('def edit_summary(log_text, fn):')
j = src.index('MUTATIONS = [')
k = src.index('\n]\n', j) + 3
region = src[i:k]
hdr = ('"""The 33 red-control mutations of the harness\'s reference validator (void_check.py), extracted VERBATIM from\n'
       'D:/tmp/rapier-parity/gate/analyze.py (section "gv"; lines %d-%d of the file as of 2026-09-30, harness rev 2). A byte-for-byte\n'
       'excerpt so the window can re-run them without importing analyze.py (a script that runs the whole harness gate and rewrites the\n'
       'harness dir). Each entry: (name, rules it must turn red, rules it may also turn red, fn(ctx) -> mutates the ctx dict in place).\n'
       'ctx keys: pins arm cfg workers log_text csv_text pose_raw fixture_raw exe_sha256 rc timed_out."""\n'
       'import json\n\n\n') % (src[:i].count('\n') + 1, src[:k].count('\n'))
open('gate/rapier/mutations_ref.py', 'w', encoding='utf-8', newline='\n').write(hdr + region)
print('extracted', region.count('\n'), 'lines')
