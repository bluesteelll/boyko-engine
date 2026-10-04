import sys
P = 'tools/window9a_run.py'
b = open(P, encoding='utf-8', newline='').read()
startmark = "    seq = 1\n    bad_runs = []\n"
start = b.index(startmark)
endmark = "    log(f'{ptag}: done')\n    return 'ok'\n"
end = b.index(endmark) + len(endmark)
assert b.count(startmark) == 1 and b.count(endmark) == 1
NEW = '''    seq = 1
    slots = {}   # (round, row, binary, W) -> {'seq', 'rec' (the LATEST attempt), 'n_rerun'}
    state['rounds_done'] = 0
    for rnd in range(rounds):
        round_recs[rnd] = {}
        for rid, key, w in order:
            ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} r{rnd} {rid}#{key}@W{w}')
            rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, seq, 'original', pdir, receipt, env, sampler)
            inject_hot(rec, rid, key, w, rnd, 0)
            append(runs_path, rec)
            state['last'] = f'{ptag} r{rnd} {rid}#{key}@W{w}'
            if 'aborted' not in rec:
                counters['timed'] += 1
            log(f'  {ptag} r{rnd} [{seq:3d}] {rid}#{key} W={w}: {tag_of(rec)}')
            if st in ('abort', 'void'):
                return (st, rec.get('aborted') or rec.get('void'))
            round_recs[rnd][(rid, key, w)] = rec
            slots[(rnd, rid, key, w)] = {'seq': seq, 'rec': rec, 'n_rerun': 0}
            seq += 1
        if has_r4:
            counters['r4_differ'] = counters.get('r4_differ', 0) + sum(
                1 for r in r4_compare(block, p, attempt, rnd, round_recs[rnd], runs_path) if r['verdict'] == 'DIFFER')
        state['rounds_done'] = rnd + 1
    # Ruling 8: the re-run STAGE. Wave a re-runs every slot whose latest attempt is contaminated or invalid; a slot leaves the
    # stage at its first clean valid attempt. Bounds ("the pass ends"): RERUN_MAX_WAVES waves, or RERUN_STAGE_FRAC x the pass's
    # own timed wall spent on re-runs; the cutoff and the STOP flag (ensure_time) end everything as before.
    stage_t0 = time.time()
    allowance_s = RERUN_STAGE_FRAC * rounds * block_estimate(block)['per_round_s']
    wave = 0
    ended = None
    while ended is None:
        todo = pending_reruns(slots)
        if not todo:
            break
        wave += 1
        if wave > RERUN_MAX_WAVES:
            ended = f'{RERUN_MAX_WAVES} waves spent'
            break
        log(f'{ptag}: re-run wave {wave}: {len(todo)} slot(s) unclean: '
            f'{[(k[0], k[1], k[2], k[3], slot_bad(s["rec"])) for k, s in todo]}')
        for (rnd, rid, key, w), s in todo:
            if not TEST and time.time() - stage_t0 + est_s(rid, key, w) + RECEIPT_S > allowance_s:
                ended = f'allowance spent ({RERUN_STAGE_FRAC} x pass wall = {allowance_s:.0f} s)'
                break
            ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} rerun {rid}#{key}@W{w}')
            why = slot_bad(s['rec'])
            n = s['n_rerun'] + 1
            rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, s['seq'], 'rerun', pdir, receipt, env, sampler,
                                       rerun_no=n)
            inject_hot(rec, rid, key, w, rnd, n)
            rec['rerun_reason'] = why
            append(runs_path, rec)
            if 'aborted' not in rec:
                counters['timed'] += 1
                counters['rerun'] += 1
            log(f'  {ptag} r{rnd} [{s["seq"]:3d}] {rid}#{key} W={w} RERUN {n} ({why}): {tag_of(rec)}')
            if st in ('abort', 'void'):
                return (st, rec.get('aborted') or rec.get('void'))
            s['rec'] = rec
            s['n_rerun'] = n
            if has_r4 and (ROWS[rid].get('r4_twin') or any(r.get('r4_twin') == rid for r in ROWS.values())):
                round_recs[rnd][(rid, key, w)] = rec  # the pair is compared again with the re-run in place
                pair = {k: v for k, v in round_recs[rnd].items() if k[2] == w and k[1] == key and
                        (k[0] == rid or ROWS[k[0]].get('r4_twin') == rid or ROWS[rid].get('r4_twin') == k[0])}
                counters['r4_differ'] = counters.get('r4_differ', 0) + sum(
                    1 for r in r4_compare(block, p, attempt, rnd, pair, runs_path) if r['verdict'] == 'DIFFER')
    cells = passcell_table(slots)
    for (rid, key, w), c in sorted(cells.items()):
        append(runs_path, {'passcell': True, 'run_tag': RUN_TAG, 'block': bname, 'pass': p, 'pass_attempt': attempt,
                           'pc_row': rid, 'pc_binary': key, 'pc_W': w, 'k_target': c['target'], 'k_clean': c['clean'],
                           'reruns': c['reruns'], 'unclean_rounds': c['unclean_rounds'],
                           'gates': c['clean'] >= MIN_K_GATE, 'short': c['clean'] < c['target'], 'time': D.now()})
    short = [(rid, key, w, c['clean'], c['target']) for (rid, key, w), c in sorted(cells.items()) if c['clean'] < c['target']]
    if wave or short:
        log(f'{ptag}: re-run stage ended after {wave} wave(s)' + (f' ({ended})' if ended else '') + f'; {len(short)} pass-cell(s) '
            f'short of their slots (k_clean < k_target; K < {MIN_K_GATE} does not gate): {short}')
    if short:
        progress(f'{block["item"]} {bname} p{p}: {len(short)} pass-cell(s) with unclean slots after the re-run stage: '
                 + ', '.join(f'{rid}#{key}@W{w} K={kc}/{kt}' for rid, key, w, kc, kt in short))
    log(f'{ptag}: done')
    return 'ok'
'''
b = b[:start] + NEW + b[end:]
helpers = '''def slot_bad(rec):
    """None when the attempt is clean and valid, else its reason ('contaminated' before 'invalid', as in window 8b)."""
    if rec.get('contaminated'):
        return 'contaminated'
    if not rec.get('valid'):
        return 'invalid'
    return None


def pending_reruns(slots):
    """The slots the next re-run wave takes: those whose LATEST attempt is not clean and valid, in original order (pure)."""
    return sorted(((k, s) for k, s in slots.items() if slot_bad(s['rec'])), key=lambda kv: kv[1]['seq'])


def passcell_table(slots):
    """(row, binary, W) -> the pass-cell's slot census: target = slots scheduled, clean = slots whose latest attempt is clean
    and valid (K), reruns = re-runs spent, unclean_rounds = the rounds still unclean (pure)."""
    cells = {}
    for (rnd, rid, key, w), s in slots.items():
        c = cells.setdefault((rid, key, w), {'target': 0, 'clean': 0, 'reruns': 0, 'unclean_rounds': []})
        c['target'] += 1
        c['reruns'] += s['n_rerun']
        if slot_bad(s['rec']) is None:
            c['clean'] += 1
        else:
            c['unclean_rounds'].append(rnd)
    return cells


def inject_hot(rec, rid, key, w, rnd, attempt_no):
    """--test only (W9A_TEST_HOT): the first n attempts of a slot are recorded as contaminated, so the re-run rule is rehearsed
    end to end with real processes. Never active outside --test."""
    if TEST and attempt_no < INJECT.get((rid, key, w, rnd), 0):
        rec['contaminated'] = True
        rec['injected_hot'] = True


'''
marker = "def run_pass(block, p, attempt, runs_path, env, sampler, counters, state):\n"
assert b.count(marker) == 1
b = b.replace(marker, helpers + marker)
old = "    rounds = 1 if TEST else block_rounds(block)\n"
assert b.count(old) == 1
b = b.replace(old, "    rounds = ARGS.test_rounds if TEST else block_rounds(block)\n")
open(P, 'w', encoding='utf-8', newline='').write(b)
print('p3 applied')
