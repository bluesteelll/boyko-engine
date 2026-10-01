"""Pass-void check (rows8b protocol void_pass_on_build_process) over every omega process incl. warm-ups + window log."""
import os
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
hits = []
for p in procs:
    for k in ('receipt_before', 'receipt_after'):
        pr = (p.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane') or (p.get(k) or {}).get('build_procs_busy'):
            hits.append((p['block'], p['pass'], p['row'], p['attempt'], k))
    if p.get('build_proc_during') or p.get('void_names_new_during'):
        hits.append((p['block'], p['pass'], p['row'], p['attempt'], 'during'))
print('omega processes (incl. warm-ups):', len(procs), 'build/lane presence or build-during hits:', hits)
log = open(os.path.join(L.RAW, 'window_log.txt'), encoding='utf-8', errors='replace').read().splitlines()
kw = [ln for ln in log if ('omega' in ln) and any(w in ln.upper() for w in ('VOID', 'ABORT', 'STOP', 'SKIP'))]
print('window_log omega lines with VOID/ABORT/STOP/SKIP:', len(kw))
for ln in kw:
    print('  ', ln[:200])
