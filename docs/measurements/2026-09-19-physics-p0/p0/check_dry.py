"""Structural checks over a dry pass: no timing is printed."""
import csv, json, os, sys
out = sys.argv[1]
recs = [json.loads(l) for l in open(os.path.join(out, 'runs.jsonl'), encoding='utf-8')]
def cols(path):
    rows = list(csv.reader(open(path, encoding='utf-8', newline='')))
    hdr = rows[0]
    return hdr, [dict(zip(hdr, r)) for r in rows[1:]]
bad = []
print('runs', len(recs), 'skipped', sum('skipped' in r for r in recs), 'nonzero exits', [(r['row'], r['W'], r.get('exit')) for r in recs if r.get('exit') not in (0,)])
contam = sorted({n for r in recs for k in ('receipt_before','receipt_after') for n in (r.get(k) or {}).get('build_procs_busy', [])})
print('build processes seen busy in dry-pass receipts:', contam)
for r in recs:
    if r['engine'] != 'boyko' or 'skipped' in r: continue
    s = r['summary']; hdr, rows = cols(r['csv'])
    tag = f"{r['row']}@W{r['W']}"
    exp_steps = s['steps']
    if len(rows) != exp_steps: bad.append(f'{tag}: csv rows {len(rows)} != steps {exp_steps}')
    if s['armed']:
        voids = sum(int(x['void']) for x in rows)
        nz = [h for h in hdr if h.endswith('_n')]
        # every span/counter column must have recorded something on some step (armed samples > 0)
        zero_cols = [h for h in nz if all(int(x[h]) == 0 for x in rows)]
        sleep_cols = {'phys_sleep_begin_n','phys_sleep_freeze_n','phys_sleep_end_n'}
        solve_cols = {h for h in nz if h.startswith('phys_') and h not in ('phys_np_pairs_n','phys_np_manifolds_n','phys_np_points_n','phys_bp_pairs_n')}
        allowed_zero = set()
        if not s['config']['sleeping']: allowed_zero |= sleep_cols
        if s['solver'] == 'reference': allowed_zero |= solve_cols
        unexpected_zero = [h for h in zero_cols if h not in allowed_zero]
        per_step = {}
        for h in ('phys_solve_build_n','phys_gravity_n','phys_warm_apply_n','phys_integrate_n','phys_pass_biased_n','phys_pass_relax_n','phys_restitution_n','phys_store_n','phys_write_back_n','phys_sleep_begin_n','phys_sleep_freeze_n','phys_sleep_end_n'):
            per_step[h.replace('_n','')] = sorted({int(x[h]) for x in rows})
        sys_n = sorted({int(x[h]) for x in rows for h in hdr if h.startswith('sys_') and h.endswith('_n')})
        waves = sorted({int(x['waves']) for x in rows}); wide = sorted({int(x['wide_colors']) for x in rows})
        wave_ok = all(int(x['waves']) == 12 * int(x['wide_colors']) for x in rows)
        narrow_ok = all(int(x['phys_color_narrow_n']) == 12 * (int(x['colors']) - int(x['wide_colors'])) for x in rows) if s['solver']=='colored' else None
        print(f"{tag:14s} armed void_steps={s['void_steps']} csv_void={voids} drops={s['drops_total']} sys_span_counts={sys_n} per_step={per_step} waves={waves} wide_colors={wide} waves==12*wide:{wave_ok} narrow==12*(colors-wide):{narrow_ok} disp_on_solve={s['threads']['solve_on_dispatcher_steps']}/{s['threads']['armed_steps']} unexpected_zero_cols={unexpected_zero}")
        if voids or s['void_steps'] or unexpected_zero or not wave_ok: bad.append(tag)
    else:
        print(f"{tag:14s} disarmed void_steps={s['void_steps']} disarmed_ring_traffic={s['disarmed_ring_traffic']} profile={s['profile_name']} zones_compiled={s['zones_compiled']}")
        if s['void_steps'] or s['disarmed_ring_traffic'] != 0: bad.append(tag)
    if s['bodies'] != (16 if s['scene']=='s16' else 1240): bad.append(f'{tag}: bodies {s["bodies"]}')
# canary
for r in recs:
    if r['row'] == 'J-C':
        s = r['summary']; hdr, rows = cols(r['csv'])
        n = sorted({int(x['sys_parity_canary_n']) for x in rows})
        ratio_ok = all(int(x['sys_parity_canary_ns']) >= s['canary_ns'] for x in rows)
        print(f"J-C@W{r['W']}: canary_ns set={s['canary_ns'] is not None and s['canary_ns']>0} canary span column present={'sys_parity_canary_ns' in hdr} span samples/step={n} span>=spin target on every step={ratio_ok} canary system position in schedule={[h for h in hdr if h.startswith('sys_') and h.endswith('_ns')]}")
# jolt
for r in recs:
    if r['engine'] != 'jolt' or 'skipped' in r: continue
    j = r['jolt']; files = r.get('files', [])
    extra = ''
    if r.get('per_frame'):
        hdr, rows = cols(r['per_frame']); extra = f"per_frame rows={len(rows)} cols={[h.strip() for h in hdr]}"
    if r.get('receipt_csv'):
        hdr, rows = cols(r['receipt_csv']); extra += f" receipt rows={len(rows)} cols={[h.strip() for h in hdr]}"
    htm = [f for f in files if f.endswith('.html')]
    print(f"{r['row']}@W{r['W']}: patch_line={'yes' if j['patch_line'] else 'NO'} stat_lines={[(x['quality'], x['threads']) for x in j['stat_lines']]} html={len(htm)} {extra}")
    if r['row'] == 'JOLT-T' and r['W'] == 1: print('   patch line:', j['patch_line'])
print('BAD:', bad)
