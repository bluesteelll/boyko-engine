"""G0: validity, cleanliness, slot selection and pass-cell K for C4-G4 and C4-G4-kd (run 033740), placement receipts.
Independent of the driver flags (which are compared afterwards, never used)."""
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_g4 as G  # noqa: E402

out = G.L.Out()
recs = G.all_records()
for block in ('C4-G4', 'C4-G4-kd'):
    procs, used, dropped = G.select(block, recs)
    pdone = sorted(k for k in G.closed_passes(recs) if k[0] == block)
    pcs = [r for r in recs if r.get('passcell') and r.get('block') == block and r.get('run_tag') == G.RUN_TAG]
    allrec = [r for r in recs if r.get('block') == block and 'row' in r]
    out('## %s' % block)
    out('records with a row: %d (run tags %s); closed passes %s; processes in closed passes %d; used %d; dropped %d'
        % (len(allrec), sorted({r.get('run_tag') for r in allrec}), [k[1] for k in pdone], len(procs), len(used),
           len(dropped)))
    out('passcell records: %s' % [(r['pass'], r['k_clean'], r['reruns'], r['gates']) for r in pcs])
    for p in procs:
        r = p['rec']
        pl = r.get('placement') or {}
        out('- p%d r%d %s seq %d: valid %s (driver %s), clean %s (driver contaminated %s); ids %d; printed/sample.json '
            'worst %.1e, estimates.json mean/sample.json worst %.1e, driver est_ms worst %.1e; kernel receipts %d '
            'sizes x families; receipts before/after %s/%s %%, witness %s %%, build during %s; wall %.1f s, %s .. %s; '
            'placement: top CPU %s at %.1f %%, main_share_top_est %s, main_cycle_frac %s'
            % (p['pass'], p['round'], p['attempt'], p['seq'], not p['valid_why'], p['driver_valid'],
               not p['clean_why'], p['driver_contaminated'], len(p['v']), p['w_pr'], p['w_est'], p['w_dr'],
               len(p['kr']), (r.get('receipt_before') or {}).get('cpu_avg'),
               (r.get('receipt_after') or {}).get('cpu_avg'), r.get('others_busy_pct'), r.get('build_proc_during'),
               r.get('wall_s'), r.get('start')[11:19], r.get('end')[11:19], pl.get('top_cpu'),
               (pl.get('top3') or [[None, float('nan')]])[0][1], pl.get('main_share_top_est'),
               pl.get('main_cycle_frac')))
        if p['valid_why'] or p['clean_why']:
            out('    problems: %s %s' % (p['valid_why'], p['clean_why']))
        agree = (not p['valid_why']) == bool(p['driver_valid']) and (not p['clean_why']) == (not p['driver_contaminated'])
        if not agree:
            out('    DISAGREES with the driver flags')
    # iteration counts per id (Flat: one iters value per id per process), for the record
    its = {}
    for p in used:
        for i, n in p['n_iters'].items():
            its.setdefault(i, set()).add(n)
    out('Flat iters per sample, range over used processes (per id): min %d, max %d' % (
        min(min(v) for v in its.values()), max(max(v) for v in its.values())))
    # kernel receipt: pairs per (family, size) identical across processes
    pairs = {}
    for p in used:
        for k, v in p['kr'].items():
            pairs.setdefault(k, set()).add(v['LeafList']['pairs'])
    multi = {k: v for k, v in pairs.items() if len(v) != 1}
    out('kernel receipts: %d (family, size) keys; pair counts identical across processes: %s' % (
        len(pairs), not multi))
    out('pairs at the G4 sizes: ' + '; '.join('%s %s' % (f, ' '.join('%d:%d' % (n, next(iter(pairs[(f, str(n))])))
                                                               for n in G.SIZES)) for f in G.FAMS))
    out('')
out.save('g0_validity.txt')
