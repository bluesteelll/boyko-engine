"""Selection + validity recomputed from raw/ (runs.jsonl receipts, each process's stdout.txt)."""
import collections
import sys
sys.dont_write_bytecode = True
import vn_lib as L

print('exe sha256 on disk:', L.sha256_file(L.EXE), '== pin:', L.sha256_file(L.EXE) == L.PIN)
procs = L.load_procs()
warm = [p for p in procs if p.get('attempt') == 'warmup']
timed = [p for p in procs if p.get('attempt') != 'warmup']
print('records: warm-ups', len(warm), 'timed', len(timed),
      'by block', dict(collections.Counter(p['block'] for p in timed)),
      'timed flag all true:', all(p.get('timed') for p in timed))
slots, used, dropped = L.select(procs)
inval = [p for p in timed if p['_valid']]
print('invalid timed processes (my rules):', len(inval))
for p in inval:
    print('  ', p['block'], p['pass'], p['round'], p['row'], p['attempt'], p['_valid'])
agree_valid = sum(1 for p in timed if bool(p['_valid']) == (p.get('valid') is False))
agree_cont = sum(1 for p in timed if bool(p['_clean']) == bool(p.get('contaminated')))
print('driver valid flag agrees with mine:', agree_valid, '/', len(timed))
print('driver contaminated flag agrees with my clean rule:', agree_cont, '/', len(timed))
for p in timed:
    if bool(p['_clean']) != bool(p.get('contaminated')):
        print('   disagreement', p['block'], p['pass'], p['round'], p['row'], p['attempt'], p['_clean'],
              p.get('contaminated'), p.get('contaminated_before'), p.get('contaminated_after'),
              p.get('contaminated_during'), p.get('contaminated_witness'))
dirty = [p for p in timed if p['_clean']]
print('dirty timed processes:', len(dirty))
print('slots', len(slots), 'used', len(used), 'originals used', sum(p['attempt'] == 'original' for p in used),
      're-runs used', sum(p['attempt'] == 'rerun' for p in used), 'dropped', len(dropped))
for k, ps in dropped:
    print('  DROPPED', k, [(p['attempt'], p['_clean']) for p in ps])
per = collections.Counter((p['block'], p['row'], p['pass']) for p in used)
for k in sorted(per):
    print('  used', k, per[k])
top_after = collections.Counter()
top_wit = collections.Counter()
for p in dirty:
    t5 = (p.get('receipt_after') or {}).get('top5') or []
    if t5:
        top_after[t5[0]['name']] += 1
    w5 = p.get('others_top5') or []
    if w5:
        top_wit[w5[0]['name']] += 1
print('dirty: top after-receipt process', dict(top_after))
print('dirty: top witness process', dict(top_wit))
ub = [(p['receipt_before'] or {}).get('cpu_avg') for p in used]
ua = [(p['receipt_after'] or {}).get('cpu_avg') for p in used]
uw = [p['others_busy_pct'] for p in used]
print('used receipts: before max', max(ub), 'after max', max(ua), 'witness max', max(uw))
print('placement present on used:', sum(1 for p in used if p.get('placement')), '/', len(used))
v1 = [p for p in procs if p['block'] == 'omega-v1-cont']
print('v1-cont start/end:', min(p['start'] for p in v1), max(p['end'] for p in v1))
print('v1-cont warm-up witness:', [p['others_busy_pct'] for p in v1 if p['attempt'] == 'warmup'])
