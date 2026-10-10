"""Second selftest patch (2026-09-30): an end-to-end WIRING case for the Rapier validator. Without it nothing proves that run_one applies
validate_rapier to a real process record (the unit cases call validate_rapier directly), so a run_one that skipped it would stay green."""
p = 'tools/selftest9a.py'
s = open(p, encoding='utf-8').read()
old = """                       for r in rprocs + rwarm) else ['args or exe differ'], False)
else:
    print('SKIP end-to-end re-run cases (--no-e2e)')"""
new = """                       for r in rprocs + rwarm) else ['args or exe differ'], False)
        # WIRING: the same rehearsal with the simd4 exe under the simd8 key (arm simd8): every real process must come back INVALID with V4
        # (features, arm, lanes and exe sha256 of the simd8 pins against a simd4 exe) - proof run_one applies the validator to a real record.
        ov_real = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
        ov_bad = {'_comment': 'REHEARSAL ONLY (selftest9a.py wiring case): the simd4 exe under the simd8 key', 'rapier': ov_real['rapier'],
                  'binaries': {'rs8': dict(ov_real['binaries']['rs4'], arm='simd8')},
                  'blocks': [{'name': 'C4-RAPIER', 'item': 'Rapier (wiring rehearsal)', 'priority': 1.5, 'warmup': ['RAP-w', 'rs8', 8]}],
                  'rows': [dict(next(r for r in ov_real['rows'] if r['id'] == 'RP-M'), id='RAP-w', blocks=['C4-RAPIER'], binaries=['rs8'], pose_ref={'rs8': 'RM8'})]}
        bad_path = os.path.join(GATE, 'selftest_tmp', 'rapier_wiring_overlay.json')
        with open(bad_path, 'w', encoding='utf-8', newline='\\n') as f:
            json.dump(ov_bad, f)
        E2E_N[0] += 1
        braw = os.path.join(GATE, 'selftest_tmp', f'e2e_rapier_wiring_{E2E_N[0]}')
        shutil.rmtree(braw, ignore_errors=True)
        os.makedirs(braw)
        e2 = dict(os.environ, W9A_EXTRA=bad_path, W9A_TEST_RAW=braw, W9A_TEST_HOT='none#none@W0:0:0')
        for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            e2.pop(k, None)
        pb = subprocess.run([sys.executable, '-B', DRIVER, '--test', '--blocks', 'C4-RAPIER', '--test-rounds', '1'], env=e2,
                            capture_output=True, text=True, cwd=W9, timeout=900)
        brecs = [json.loads(l) for l in open(os.path.join(braw, 'runs.jsonl'), encoding='utf-8')] if os.path.exists(os.path.join(braw, 'runs.jsonl')) else []
        bprocs = [r for r in brecs if 'row' in r and r.get('attempt') != 'warmup']
        case('e2e wiring: the simd4 exe under the simd8 key -> every one of the 5 real processes is INVALID with V4 (run_one applies the validator)',
             [] if (pb.returncode == 0 and len(bprocs) == 5 and all(any(x.startswith('V4:') for x in (r.get('invalid') or [])) for r in bprocs)
                    and all(r.get('valid') is False for r in bprocs)) else [f'exit {pb.returncode} procs {len(bprocs)} invalid {[r.get("invalid") for r in bprocs][:1]} {pb.stderr[-200:]}'], False)
        cn = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'counts9a.py'), braw], capture_output=True, text=True).stdout.splitlines()
        case('e2e wiring: counts9a.py prints an INVALID line (with the rule V4) for a rapier process the validator rejected',
             [] if any(l.startswith('INVALID') and 'V4:' in l for l in cn) else ['no INVALID V4 line'], False)
else:
    print('SKIP end-to-end re-run cases (--no-e2e)')"""
assert s.count(old) == 1
s = s.replace(old, new)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patched')
