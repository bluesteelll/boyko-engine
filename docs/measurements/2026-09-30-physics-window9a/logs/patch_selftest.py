"""One-off patch of tools/selftest9a.py for the Rapier block (2026-09-30). Run from the window dir:  python -B logs/patch_selftest.py"""
p = 'tools/selftest9a.py'
s = open(p, encoding='utf-8').read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:90])
    s = s.replace(old, new)


# ---- docstring
rep('''A red counts only for its NAMED reason (only_for) where a mutation could trip an unrelated rule."""''',
    '''A red counts only for its NAMED reason (only_for) where a mutation could trip an unrelated rule.
2026-09-30 (the Rapier block C4-RAPIER, section 6): its structure and pins, the V1-V9 validator on the 20 real gate cells and on mutated copies,
an end-to-end --test rehearsal of the block (section 7), and the overlay placement cases over a GENERATED overlay (section 8)."""''')

# ---- section 1: block order and K per block (Rapier merged, BR at K 3)
rep('''case('blocks run in priority order AB, G5, BR, G4, G4-kd (merge gate first; BR precedes G4 as ruling 5 reads)',
     [] if [b['name'] for b in R.BLOCKS] == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'] else [str([b['name'] for b in R.BLOCKS])], False)''',
    '''case('blocks run in priority order AB, RAPIER, G5, BR, G4, G4-kd (merge gate first; Rapier after C4-AB and before C4-G5; BR precedes G4 as ruling 5 reads)',
     [] if [b['name'] for b in R.BLOCKS] == ['C4-AB', 'C4-RAPIER', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'] else [str([b['name'] for b in R.BLOCKS])], False)''')
rep('''case('K per cell: C4-BR 9, C4-G4 9, C4-G4-kd 3, C4-AB 9, C4-G5 9',
     [] if ks == {'C4-BR': 9, 'C4-G4': 9, 'C4-G4-kd': 3, 'C4-AB': 9, 'C4-G5': 9} else [str(ks)], False)''',
    '''case('K per cell: C4-BR 3 (diagnostic, ruling (b)), C4-G4 9, C4-G4-kd 3, C4-AB 9, C4-G5 9, C4-RAPIER 9',
     [] if ks == {'C4-BR': 3, 'C4-G4': 9, 'C4-G4-kd': 3, 'C4-AB': 9, 'C4-G5': 9, 'C4-RAPIER': 9} else [str(ks)], False)
br_blk = next(b for b in R.BLOCKS if b['name'] == 'C4-BR')
case('C4-BR is marked DIAGNOSTIC ONLY / not claim-bearing in its block note, its row note and the protocol (ruling (b))',
     [w for w, t in (('block', br_blk.get('note', '')), ('row', R.ROWS['C4-BR'].get('note', '')), ('protocol', R.PROTO.get('own_k', '')))
      if 'claim-bearing' not in t or ('DIAGNOSTIC' not in t.upper())], False)
case('C4-BR runs ONE pass of three rounds (passes 1: K 3, one reversed pass)', [] if (R.block_passes(br_blk), R.block_rounds(br_blk)) == (1, 3) else [str(br_blk)], False)''')

# ---- section 6: replaced wholesale
i = s.index('# ------------------------------------------------------------------------------------------------ 6. the reserved Rapier slot')
j = s.index('# ------------------------------------------------------------------------------------------------ 7. end-to-end re-run rule')
s6 = open('logs/selftest_s6.txt', encoding='utf-8').read()
s = s[:i] + s6 + '\n' + s[j:]

# ---- section 7: an end-to-end rehearsal of C4-RAPIER (inside the E2E block, before its else)
rep('''else:
    print('SKIP end-to-end re-run cases (--no-e2e)')''', '''    # ---- the Rapier block end to end (2026-09-30): --test rehearses control flow with real 12-step Rapier processes on the REAL overlay
    if have_rap:
        E2E_N[0] += 1
        rraw = os.path.join(GATE, 'selftest_tmp', f'e2e_rapier_{E2E_N[0]}')
        shutil.rmtree(rraw, ignore_errors=True)
        os.makedirs(rraw)
        e = dict(os.environ, W9A_TEST_RAW=rraw, W9A_TEST_HOT='RP-D#rs8@W4:0:1')
        e.pop('W9A_EXTRA', None)
        for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            e.pop(k, None)
        pr = subprocess.run([sys.executable, '-B', DRIVER, '--test', '--blocks', 'C4-RAPIER', '--test-rounds', '1'], env=e,
                            capture_output=True, text=True, cwd=W9, timeout=900)
        rrecs = [json.loads(l) for l in open(os.path.join(rraw, 'runs.jsonl'), encoding='utf-8')] if os.path.exists(os.path.join(rraw, 'runs.jsonl')) else []
        rprocs = [r for r in rrecs if 'row' in r and r.get('attempt') != 'warmup']
        rwarm = [r for r in rrecs if 'row' in r and r.get('attempt') == 'warmup']
        orig = sorted((r for r in rprocs if r['attempt'] == 'original'), key=lambda r: r['seq'])
        case('e2e C4-RAPIER --test (1 round, W9A_TEST_HOT: RP-D#rs8@W4 hot once): exit 0, 20 originals + 1 re-run, 0 invalid, every record kind rapier',
             [] if (pr.returncode == 0 and len(orig) == 20 and len(rprocs) == 21 and not any(r.get('invalid') for r in rprocs + rwarm)
                    and all(r['kind'] == 'rapier' for r in rprocs + rwarm)) else [f'exit {pr.returncode} originals {len(orig)} procs {len(rprocs)} invalid {[r.get("invalid") for r in rprocs if r.get("invalid")][:2]} {pr.stderr[-200:]}'], False)
        case('e2e C4-RAPIER: two warm-ups per pass in order RP-D rs8 W8 then RP-D rs4 W8 (one per exe)',
             [] if [(r['row'], r['binary'], r['W']) for r in rwarm] == [('RP-D', 'rs8', 8), ('RP-D', 'rs4', 8)] else [str([(r['row'], r['binary'], r['W']) for r in rwarm])], False)
        case('e2e C4-RAPIER: the EXECUTED order of pass 0 is the reversed forward order and the four configurations are adjacent at every W',
             [] if [(r['row'], r['binary'], r['W']) for r in orig] == R.pass_order(R.cell_order('C4-RAPIER'), 0, rb) else ['executed order differs'], False)
        case('e2e C4-RAPIER: the injected hot slot was re-run once (round 0, rerun_no 1) and left the stage clean',
             [] if sorted((r['round'], r['rerun_no'], r['row'], r['binary'], r['W']) for r in rprocs if r['attempt'] == 'rerun') == [(0, 1, 'RP-D', 'rs8', 4)] else ['re-run set differs'], False)
        case('e2e C4-RAPIER: the process records carry the harness command line (--cfg, --install loop, --workers W, no --receipt) and the per-binary exe',
             [] if all(('--install' in r['args'] and 'loop' in r['args'] and '--receipt' not in r['args'] and r['args'][r['args'].index('--workers') + 1] == str(r['W'])
                        and r['exe'].endswith({'rs8': 'rapier_parity_simd8_736c2a06.exe', 'rs4': 'rapier_parity_simd4_1e5136ca.exe'}[r['binary']]))
                       for r in rprocs + rwarm) else ['args or exe differ'], False)
else:
    print('SKIP end-to-end re-run cases (--no-e2e)')''')

# ---- section 8: the placement overlay is GENERATED from the real Rapier overlay (the pre-fix file had the old data-driven schema)
rep('''base_ov = json.load(open(os.path.join(W9, 'test', 'rapier_overlay', 'rows9a.extra.json'), encoding='utf-8'))
''', '''real_ov = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
rap_row = json.loads(json.dumps(next(r for r in real_ov['rows'] if r['id'] == 'RP-M')))
rap_row.update({'id': 'RAP-matched', 'blocks': ['C4-RAPIER'], 'binaries': ['rs8'], 'pose_ref': {'rs8': 'RM8'}})
base_ov = {'_comment': 'REHEARSAL ONLY, GENERATED by selftest9a.py from the real rows9a.extra.json: one Rapier row on the rs8 exe, for the placement cases below',
           'rapier': real_ov['rapier'], 'binaries': {'rs8': real_ov['binaries']['rs8']},
           'blocks': [{'name': 'C4-RAPIER', 'item': 'Rapier (placement rehearsal)', 'priority': 4.5, 'warmup': ['RAP-matched', 'rs8', 8]}],
           'rows': [rap_row]}
os.makedirs(os.path.join(W9, 'test', 'rapier_overlay'), exist_ok=True)
with open(os.path.join(W9, 'test', 'rapier_overlay', 'rows9a.extra.json'), 'w', encoding='utf-8', newline='\\n') as f:
    json.dump(base_ov, f, indent=1)
''')
rep('''case('an own block at priority 4.5 runs AFTER C4-G4 (audit-1 W2: the first cut of PREP.md said "before C4-G4")',
     [] if dry_blocks(dry(ov45, 'own_45')[1]) == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-RAPIER', 'C4-G4-kd'] else ['order'], False)''',
    '''case('an own block at priority 4.5 runs AFTER C4-G4 (audit-1 W2: the first cut of PREP.md said "before C4-G4")',
     [] if dry_blocks(dry(ov45, 'own_45')[1]) == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-RAPIER', 'C4-G4-kd'] else ['order'], False)''')
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patched')
