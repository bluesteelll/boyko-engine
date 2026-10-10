"""Builds timing_raw.md from runs.jsonl, the run logs and the §9 probe series. Pure post-processing."""
import json, os, re, statistics, sys

MQ = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(MQ, 'probe'))
import r2_window as r2  # noqa: E402

runs = [json.loads(l) for l in open(os.path.join(MQ, 'runs.jsonl'), encoding='utf-8')]
ms = lambda x: f'{x / 1e6:.4f}'
out = []
P = out.append

ARMS = {'s6': ('d5782d43', 'b74f7ee8'), 's7': ('d552be05', 'a56007ab'), 's8': ('d552be05', 'a56007ab'),
        's9': ('08fe7b9f', '8d656ad8')}
SUSPECT = {('s9', 'full_step_1', 'A', 2): 'after-receipt 16.66 % (browser burst right at run end); A2 = 20.61 ms against A1/A3/A4 = 18.66/19.49/18.97'}


def rows_of(entry, tag, rid):
    res = {}
    for r in runs:
        if r['entry'] == entry and r['tag'] == tag and rid in r['rows']:
            res[(r['arm'], r['n'])] = r['rows'][rid]['median_ns']
    return res


def stats(vals):
    if not vals:
        return None
    m = statistics.mean(vals)
    return m, (max(vals) - min(vals)) / m


def summary_line(entry, tag, rid, ns=(1, 2), exclude=()):
    d = rows_of(entry, tag, rid)
    a = [d[('A', n)] for n in ns if ('A', n) in d and ('A', n) not in exclude]
    b = [d[('B', n)] for n in ns if ('B', n) in d and ('B', n) not in exclude]
    sa, sb = stats(a), stats(b)
    cells = [' / '.join(ms(x) for x in a) or '—', ' / '.join(ms(x) for x in b) or '—']
    if sa:
        cells.append(f'{sa[1] * 100:.2f} %')
    else:
        cells.append('—')
    cells.append(f'{sb[1] * 100:.2f} %' if sb else '—')
    if sa and sb:
        ratio = sb[0] / sa[0]
        cells.append(f'{ratio:.4f}')
        cells.append(f'{min(b) / max(a):.4f} .. {max(b) / min(a):.4f}')
    else:
        cells += ['—', '—']
    return cells


def table(entry, tag, rids, ns=(1, 2), exclude=(), label=''):
    P(f'| row {label}| A medians (ms) | B medians (ms) | A/A spread | B/B spread | B/A (mean of medians) | B/A range (minB/maxA .. maxB/minA) |')
    P('|---|---|---|---|---|---|---|')
    for rid in rids:
        P(f'| `{rid}` | ' + ' | '.join(summary_line(entry, tag, rid, ns, exclude)) + ' |')
    P('')


def rec_block(r):
    lab = f"{r['arm']}{r['n']}"
    rb, ra = r['receipt_before'], r['receipt_after']
    t5 = lambda rc: ', '.join(f"{t['name']} {t['cpu_s']}" for t in rc['top5'])
    P(f"#### {r['entry']} · {r['tag']} · {lab} · {r['sha8']}{'  (SUPPLEMENTARY)' if r['n'] > 2 else ''}")
    P(f"- exe `{r['exe']}` sha256 `{r['exe_sha256'][:16]}…` (verified before the run); cwd `{r['cwd']}`")
    P(f"- args `{' '.join(r['args'])}`; env CARGO_TARGET_DIR=`{r['target_dir']}`, CRITERION_DEBUG=1; rc={r['exit_code']}; "
      f"run {r['run_start']} → {r['run_end']} ({r['run_wall_s']} s)")
    P(f"- ids that ran: {', '.join('`' + i + '`' for i in r['ids_ran'])}")
    wait = f"; waited {r['waited_s']} s (polls: " + ', '.join(f"{p['cpu_avg']} %" for p in r['before_polls']) + ')' if r['before_polls'] else ''
    P(f"- receipt BEFORE {rb['time']}: procs {rb['process_count']}, CPU 10 s avg **{rb['cpu_avg']} %** "
      f"(samples {rb['cpu_samples']}), top5 [{t5(rb)}], toolchain procs present {rb['watched_present']}{wait}")
    if 'during_run' in r:
        d = r['during_run']
        P(f"- DURING run: whole-machine busy minus the bench process = **{d['non_bench_busy_pct_of_machine']} %** "
          f"(bench itself {d['bench_pct_of_machine']} %); top5 other [{', '.join(t['name'] + ' ' + str(t['cpu_s']) for t in d['top5'])}]")
    P(f"- receipt AFTER {ra['time']}: procs {ra['process_count']}, CPU 10 s avg **{ra['cpu_avg']} %** "
      f"(samples {ra['cpu_samples']}), top5 [{t5(ra)}], toolchain procs present {ra['watched_present']}")
    if r['warnings']:
        P(f"- criterion warnings: {'; '.join(sorted(set(r['warnings'])))}")
    key = (r['entry'], r['tag'], r['arm'], r['n'])
    if key in SUSPECT:
        P(f'- **SUSPECT: {SUSPECT[key]}**')
    P('')
    P('| id | median (ms) | median 95 % CI (ms) | slope (ms) | mean (ms) | W (warm-up iters) | N (timed iters) | sampling | estimates.json LastWriteTime | copied to |')
    P('|---|---|---|---|---|---|---|---|---|---|')
    for rid in r['ids_ran']:
        x = r['rows'][rid]
        W = r['warmup'].get(rid, {}).get('W')
        P(f"| `{rid}` | {ms(x['median_ns'])} | [{ms(x['median_ci'][0])}, {ms(x['median_ci'][1])}] | "
          f"{ms(x['slope_ns']) if x.get('slope_ns') else '—'} | {ms(x['mean_ns'])} | {W} | {x['N']} "
          f"(i1={int(x['iters'][0])}) | {x['sampling_mode']} | {x['estimates_mtime']} | "
          f"`raw/{os.path.relpath(x['copied_to'], os.path.join(MQ, 'raw')).replace(os.sep, '/')}` |")
    P('')


# ---------------------------------------------------------------- header
t0 = min(r['receipt_before']['time'] for r in runs)
t1 = max(r['receipt_after']['time'] for r in runs)
P('# MEASUREMENT-QUEUE §6–§9: timing record (raw), 2026-09-18')
P('')
P(f'Tester, timing stage. {len(runs)} timed runs in total, from {t0} to {t1}: the 46 the protocol asks for, plus 16 supplementary runs (n = 3, 4; see §0.4). §1 summarises, §2 indexes every run in one row per benchmark id, §3 holds the full per-run records. '
  'No run compiled anything. Each run is a PREBUILT bench-profile exe from `exes.json`, invoked directly with no cargo involved. '
  'Machine: 16 logical CPUs; the owner declared it quiet at about 19:00, and no other workflow ran.')
P('')
P('## 0. Conditions and protocol')
P('')
P('### 0.1 Invocation')
P('- `cd <worktree>/crates/boyko_physics; CARGO_TARGET_DIR=<arm target dir> CRITERION_DEBUG=1 <exe> --bench --noplot <filter>`. '
  'The driver (`run_one.py`) removes `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CRITERION_HOME`, `CARGO` and `CARGO_CRITERION_PORT` from the environment, '
  'and before every run it checks the exe sha256 against `exes.json` (it asserts). Every exe matched.')
P('- **Filters are exact.** `full_step/1` as a criterion regex also matches `full_step/16`, so every single-row run uses `--exact <full id>`. '
  'Multi-row runs use anchored regexes (`^row_identity_churn/`, `^row_identity_churn/stable/`, `^soft_step_sp2/coupled/`, `^sleeping_pipeline/`). '
  'Every run\'s record lists the ids that actually ran. All 62 match their entry exactly, and none ran 0 rows.')
P('- Additions to the spec\'s `cargo bench -- <filter>`:')
P('  - `--noplot` skips criterion\'s plot/HTML rendering between rows. gnuplot is absent, so the fallback would be in-process plotters. The flag has no effect on sampling.')
P('  - `CRITERION_DEBUG=1` makes criterion 0.5.1 print `Completed <W> iterations` after warm-up. That is the W §9 R2 needs. It adds prints only, outside the timed region.')
P('- The value recorded is `estimates.json` `median.point_estimate` with its 95 % CI (the median of per-sample time/iteration). '
  'Criterion\'s console `time:` line is the linear-regression **slope**, which is listed beside it for reference.')
P('- `estimates.json` is copied, together with sample.json, benchmark.json and tukey.json, from `<target>/criterion/<id>/new/` to '
  '`raw/<entry>/<arm><n>/<dir>/` right after each run. The driver checks that every copied estimates.json has a LastWriteTime later than its run start. All 62 runs passed that check, so no stale file was recorded.')
P('')
P('### 0.2 Load receipts')
P('- BEFORE and AFTER every run, in PowerShell (`receipt.ps1`):')
P('  - `(Get-Process).Count`.')
P('  - The 10 × 1 s average of `\\Processor(_Total)\\% Processor Time`.')
P('  - The top 5 processes by CPU-time delta over those same 10 s. This covers the processes whose `TotalProcessorTime` is readable without elevation, about 108 of about 235.')
P('  - Whether any cargo / rustc / link / lld / dxc / clippy / miri / rust-analyzer process exists.')
P('- **Toolchain processes:** no cargo, rustc, link, lld, dxc or clippy process existed at any receipt. `rust-analyzer` was present throughout and never appeared in any top 5, so it was idle.')
P('- **Background at every receipt:** the owner\'s browser (`browser`, several processes, intermittent bursts), Task Manager, steamwebhelper, audiodg, and claude.')
P('- **DURING-run accounting** was added to the driver after the s9 full_step/1 required set, once the after-receipt of A2 had shown a burst (see 0.3). It covers s9 full_step/4, s9 awake and all supplementary runs:')
P('  - `GetSystemTimes` busy time over the run window, minus the bench process\'s own CPU (`GetProcessTimes`), as a % of the whole machine.')
P('  - The top 5 other processes by CPU delta over the run.')
P('  - Runs before that have only the before/after receipts.')
P('')
P('### 0.3 Wait rule, contamination, suspect runs')
waited = [r for r in runs if r['before_polls']]
P(f"- **Wait rule:** {len(waited)} runs had a before-receipt above 5 % and waited. Each wait was one poll of about 59 s, after which the receipt settled below 5 %: "
  + '; '.join(f"{r['entry']} {r['tag']} {r['arm']}{r['n']} (first poll {r['before_polls'][0]['cpu_avg']} %, browser top)" for r in waited)
  + '. **No run was CONTAMINATED under the before-rule.** The maximum before-receipt was '
  + f"{max(r['receipt_before']['cpu_avg'] for r in runs)} %.")
hi_after = [r for r in runs if r['receipt_after']['cpu_avg'] > 5]
P('- **After-receipts above 5 %:** ' + '; '.join(f"{r['entry']} {r['tag']} {r['arm']}{r['n']} = {r['receipt_after']['cpu_avg']} %" for r in hi_after) + '.')
P('  - **s9 full_step/1 A2 is SUSPECT.** Its after-receipt was 16.66 %: four browser processes used 15 cpu-s in 10 s, and the per-second samples ran 13–28 %. Its median, 20.61 ms, sits 6–10 % above the other three A runs (18.66 / 19.49 / 18.97).')
P('  - The other three sit at 5.1–5.6 %, all browser bursts.')
P('    - s7 full_step/1 B1 (5.61 %): its median is within 0.3 % of B2.')
P('    - s9 full_step/4 B2 (5.12 %): its median is 2.8 % above B1.')
P('    - s7 churn_stable B2 (5.4 %): its stable/sleeping_off median is the outlier described in 0.4.')
dr = [r['during_run']['non_bench_busy_pct_of_machine'] for r in runs if 'during_run' in r]
P(f"- **During-run non-bench load** over the {len(dr)} runs that have it: {min(dr)}–{max(dr)} % of the machine, about 0.3–0.7 of one core. The browser and Task Manager were the top consumers throughout.")
P('')
P('### 0.4 Supplementary runs (n = 3, 4), beyond the A1 B1 A2 B2 protocol')
P('Each set was run interleaved as A3 B3 A4 B4, immediately after the required set:')
P('- **s9 full_step/1:** replaces the suspect A2.')
P('- **s9 pyramid_sleeping_off:** the PRIMARY row. B1 and B2 differed by 3.6 %.')
P('- **s9 pyramid_awake_sleeping_on:** the A/A spread was 4.5 %.')
P('- **s7 row_identity_churn/stable:** B2 sleeping_off read 17.55 ms against 16.97–17.12 ms for the other three runs of that row, with receipts of 4.1 % before and 5.4 % after, and it alone would have moved s7 R1.')
P('The summaries in §1 list the required set (n = 1, 2) first, then the all-runs set. **Rule application is left to the results-analyst.**')
P('')
P('### 0.5 Procedural notes')
P('- The first run, s6 full_step/1 A1, was captured by the first driver version, which wrote stdout and stderr to separate files. That broke the id and W parse. Its `new/` dir was untouched when the record was re-derived from it (the next write to that target dir came from A2). The logs are kept as `.out` / `.err` / `.log`, and the superseded parses as `runs_first_parse.jsonl` / `runs_second_parse.jsonl`.')
P('- Criterion also writes a `base/` baseline in each target dir, so A2 printed a change against A1 in the same dir. Those change lines are console output only.')
P('- No file in any worktree or repository was modified. Nothing was committed or deleted. The only new files are under the scratch dir `mq/`, plus criterion\'s own `criterion/` dirs inside `D:/wt/_targets/mq-*`.')
P('')

# ---------------------------------------------------------------- summaries
P('## 1. Summaries (computed from the medians below)')
P('')
P('Definitions:')
P('- spread = (max − min) / mean, over the medians of one arm.')
P('- B/A = mean(B medians) / mean(A medians).')
P('- range = the most pessimistic and most optimistic B/A pairing.')
P('')
P('### §6 — A = d5782d43, B = b74f7ee8')
table('s6', 'full_step_1', ['jolt_parity_pyramid/full_step/1'])
table('s6', 'full_step_4', ['jolt_parity_pyramid/full_step/4'])
P('`row_identity_churn` (B only, all 12 rows, B1 and B2). Arm / `stable` ratio uses the same run\'s `stable` row with the same sleeping setting:')
P('')
P('| sleeping | arm | B1 median (ms) | B2 median (ms) | B/B spread | arm/stable B1 | arm/stable B2 | arm/stable (means) |')
P('|---|---|---|---|---|---|---|---|')
ch = [r for r in runs if r['entry'] == 's6' and r['tag'] == 'row_identity_churn_all']
b1 = next(r for r in ch if r['n'] == 1); b2 = next(r for r in ch if r['n'] == 2)
for sl in ('sleeping_off', 'sleeping_on'):
    st = f'row_identity_churn/stable/{sl}'
    for arm in ('stable', 'swap_churn', 'archetype_shift', 'first_archetype_spawn', 'burst_despawn', 'burst_migrate'):
        rid = f'row_identity_churn/{arm}/{sl}'
        x1, x2 = b1['rows'][rid]['median_ns'], b2['rows'][rid]['median_ns']
        s1, s2 = b1['rows'][st]['median_ns'], b2['rows'][st]['median_ns']
        P(f'| {sl} | {arm} | {ms(x1)} | {ms(x2)} | {abs(x1 - x2) / ((x1 + x2) / 2) * 100:.2f} % | {x1 / s1:.4f} | {x2 / s2:.4f} | {(x1 + x2) / (s1 + s2):.4f} |')
P('')
P('Structural receipt printed by every row_identity_churn run, for 4 churn steps. It was identical in all 10 logs, both s6 B runs and all 8 s7 runs on both arms:')
P('')
P('| row | maps built | rows resolved by stage 2 |')
P('|---|---|---|')
for l in open(os.path.join(MQ, 'runlogs', 's6_row_identity_churn_all_B1.log'), encoding='utf-8'):
    m = re.match(r'row_identity_churn/(\S+): 4 churn steps built (\d+) maps and resolved (\d+) rows', l)
    if m:
        P(f'| {m.group(1)} | {m.group(2)} | {m.group(3)} |')
P('')

P('### §7 — A = d552be05, B = a56007ab')
table('s7', 'full_step_1', ['jolt_parity_pyramid/full_step/1'])
table('s7', 'full_step_4', ['jolt_parity_pyramid/full_step/4'])
P('`row_identity_churn/stable`, the required set (n = 1, 2):')
P('')
table('s7', 'churn_stable', ['row_identity_churn/stable/sleeping_off', 'row_identity_churn/stable/sleeping_on'])
P('`row_identity_churn/stable`, all runs (n = 1..4, supplementary included):')
P('')
table('s7', 'churn_stable', ['row_identity_churn/stable/sleeping_off', 'row_identity_churn/stable/sleeping_on'], ns=(1, 2, 3, 4))
table('s7', 'soft_coupled', ['soft_step_sp2/coupled/64', 'soft_step_sp2/coupled/256'])
P('§7 R4 (the alloc census / attribution) is structural. It was not run here and is still outstanding (see the build report).')
P('')

P('### §8 — A = d552be05 + port (contact_wakes helper = 0), B = a56007ab')
table('s8', 'sp_all', ['sleeping_pipeline/pyramid_sleeping_off', 'sleeping_pipeline/pyramid_awake_sleeping_on',
                       'sleeping_pipeline/pyramid_frozen_sleeping_on'])
P('- **Frozen-arm receipts.** All 4 s8 logs print `froze at step 61, manifolds 8555, islands 1` and `contact_wakes 0`. Arm A\'s 0 is the stub; arm B\'s is the real counter.')
P('  - R3: freeze step, manifolds and islands are the same on both trees.')
P('  - R4: the manifold count is not 0.')
P('  - The same receipt appears in all 16 s9 sleeping_pipeline logs, because setup always builds the frozen pile.')
P('- **`jolt_parity_pyramid/full_step/1` cross-check: REUSED from §7, not re-run.** It is the same pair of arms (d552be05 / a56007ab) and the same binaries: exe sha256 `aa70f3e2…` on A and `662a9685…` on B.')
P('  - The d552be05 bench exe was built before the s8 port and was byte-identical after it (build report §1).')
P('  - So §7\'s full_step/1 table above is §8\'s cross-check row.')
P('')

P('### §9 — A = 08fe7b9f, B = 8d656ad8')
P('`pyramid_sleeping_off` (PRIMARY), `full_step/1`, `full_step/4` and `pyramid_awake_sleeping_on`, required set (n = 1, 2):')
P('')
for tag, rid in [('sp_off', 'sleeping_pipeline/pyramid_sleeping_off'), ('full_step_1', 'jolt_parity_pyramid/full_step/1'),
                 ('full_step_4', 'jolt_parity_pyramid/full_step/4'), ('sp_awake', 'sleeping_pipeline/pyramid_awake_sleeping_on')]:
    table('s9', tag, [rid])
P('All runs (n = 1..4). The full_step/1 row is shown with and without the suspect A2:')
P('')
table('s9', 'sp_off', ['sleeping_pipeline/pyramid_sleeping_off'], ns=(1, 2, 3, 4))
table('s9', 'full_step_1', ['jolt_parity_pyramid/full_step/1'], ns=(1, 2, 3, 4), label='(n=1..4, incl. A2) ')
table('s9', 'full_step_1', ['jolt_parity_pyramid/full_step/1'], ns=(1, 2, 3, 4), exclude=(('A', 2),), label='(n=1..4, A2 excluded) ')
table('s9', 'sp_awake', ['sleeping_pipeline/pyramid_awake_sleeping_on'], ns=(1, 2, 3, 4))
P('**The sleeping-on row (`pyramid_awake_sleeping_on`) is NOT like for like** (§9: arm A\'s gravity-on pile never freezes and arm B\'s does).')
P('- This bench arm uses `sleep_threshold = 0`, so neither arm latches.')
P('- Beside the primary number only. It is never S5\'s price.')
P('')

# ---- §9 R2
P('#### §9 R2: live contact points over each run\'s OWN timed steps')
P('')
P('Source: the probe series `probe/sp_<sha8>.csv` and `probe/jolt_<sha8>.csv`, from the build report §5b, evaluated with `probe/r2_window.py`\'s functions over each run\'s own window:')
P('- **sleeping_pipeline:** steps [31+W, 30+W+N].')
P('- **full_step:** every sample restarts at step 21. Linear d = i1 (the first sample\'s iteration count from sample.json).')
P('')
P('| row | run | W | N | sampling | timed steps | mean live points / timed step |')
P('|---|---|---|---|---|---|---|')
pts = {}
for r in runs:
    if r['entry'] != 's9':
        continue
    rid = r['ids_ran'][0]
    x = r['rows'][rid]
    W = r['warmup'][rid]['W']
    series_arm = rid.split('/', 1)[1]
    if r['bench'] == 'sleeping_pipeline':
        s = r2.load(MQ, 'sp', r['sha8'])[series_arm]
        m, win = r2.sp_mean(s, W, x['N'])
        mode = 'linear' if x['sampling_mode'] == 'Linear' else 'flat'
    else:
        s = r2.load(MQ, 'jolt', r['sha8'])[series_arm]
        d = int(x['iters'][0])
        mode = 'linear' if x['sampling_mode'] == 'Linear' else 'flat'
        assert mode == 'linear' and x['iters'] == [float(j * d) for j in range(1, 21)]
        m, win = r2.jolt_mean(s, mode, d)
    pts[(r['tag'], r['arm'], r['n'])] = m
    P(f"| `{rid}` | {r['arm']}{r['n']} ({r['sha8']}) | {W} | {x['N']} | {mode} d={int(x['iters'][0])} | {win[0]}–{win[1]} | {m:.1f} |")
P('')
P('| row | set | A pts/step (mean over runs) | B pts/step (mean over runs) | row ratio B/A (points) | time ratio B/A (medians) | time/row |')
P('|---|---|---|---|---|---|---|')
for tag, rid in [('sp_off', 'sleeping_pipeline/pyramid_sleeping_off'), ('full_step_1', 'jolt_parity_pyramid/full_step/1'),
                 ('full_step_4', 'jolt_parity_pyramid/full_step/4'), ('sp_awake', 'sleeping_pipeline/pyramid_awake_sleeping_on')]:
    for label, ns, excl in [('n=1,2', (1, 2), ()), ('n=1..4', (1, 2, 3, 4), ()), ('n=1..4 excl. A2', (1, 2, 3, 4), (('A', 2),))]:
        if tag == 'full_step_4' and label != 'n=1,2':
            continue
        if label == 'n=1..4 excl. A2' and tag != 'full_step_1':
            continue
        d = rows_of('s9', tag, rid)
        A = [n for n in ns if ('A', n) in d and ('A', n) not in excl]
        B = [n for n in ns if ('B', n) in d and ('B', n) not in excl]
        pa = statistics.mean(pts[(tag, 'A', n)] for n in A); pb = statistics.mean(pts[(tag, 'B', n)] for n in B)
        ta = statistics.mean(d[('A', n)] for n in A); tb = statistics.mean(d[('B', n)] for n in B)
        P(f'| `{rid}` | {label} | {pa:.1f} | {pb:.1f} | {pb / pa:.4f} | {tb / ta:.4f} | {(tb / ta) / (pb / pa):.4f} |')
P('')
P('- **The two arms timed different windows.**')
P('  - `pyramid_sleeping_off`: every A run chose W=255, N=420 (steps 286–705), and every B run chose W=127, N=210 (steps 158–367). B also drew criterion\'s "Unable to complete 20 samples in 5.0s" warning in all 4 runs.')
P('  - `full_step/4`: B1 chose linear d=2 where every other run chose d=3.')
P('  - The row ratios above are therefore per arm and per run, as the build report\'s FLAG R2-1 / R2-2 prescribe.')
P('')

# ---------------------------------------------------------------- compact index
P('## 2. Every run, compact index (execution order)')
P('')
P('Column key:')
P('- **before / after:** `process count, 10 s CPU avg, top consumer (cpu-s over the 10 s)`.')
P('- **during:** whole-machine busy minus the bench process, where recorded.')
P('- **mtime:** the LastWriteTime of the copied `estimates.json`.')
P('')
P('The full per-run blocks (exe path, sha256, args, all five top processes, raw per-second samples, copy path) are in §3.')
P('')
P('| # | entry | run | sha8 | id | median ms [95 % CI] | W / N | before | after | during | mtime |')
P('|---|---|---|---|---|---|---|---|---|---|---|')
for k, r in enumerate(runs, 1):
    rb, ra = r['receipt_before'], r['receipt_after']
    top = lambda rc: f"{rc['top5'][0]['name']} {rc['top5'][0]['cpu_s']}"
    dur = f"{r['during_run']['non_bench_busy_pct_of_machine']} %" if 'during_run' in r else '—'
    flag = ' **SUSPECT**' if (r['entry'], r['tag'], r['arm'], r['n']) in SUSPECT else ''
    sup = ' (supp.)' if r['n'] > 2 else ''
    for j, rid in enumerate(r['ids_ran']):
        x = r['rows'][rid]
        W = r['warmup'].get(rid, {}).get('W')
        lead = (f"| {k} | {r['entry']} | {r['arm']}{r['n']}{sup}{flag} | {r['sha8']} | " if j == 0 else '| | | | | ')
        rec_cols = (f"{rb['process_count']}, {rb['cpu_avg']} %, {top(rb)} | {ra['process_count']}, {ra['cpu_avg']} %, {top(ra)} | {dur}"
                    if j == 0 else '〃 | 〃 | 〃')
        P(lead + f"`{rid}` | {ms(x['median_ns'])} [{ms(x['median_ci'][0])}, {ms(x['median_ci'][1])}] | {W} / {x['N']} | "
          + rec_cols + f" | {x['estimates_mtime'][11:23]} |")
P('')

# ---------------------------------------------------------------- per-run
P('## 3. Every run in full, in execution order')
P('')
for r in runs:
    rec_block(r)

open(os.path.join(MQ, 'timing_raw.md'), 'w', encoding='utf-8').write('\n'.join(out) + '\n')
print(len(out), 'lines written')
