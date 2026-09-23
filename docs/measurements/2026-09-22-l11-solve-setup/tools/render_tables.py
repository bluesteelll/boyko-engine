"""Render the window-3 tables from raw/analysis.json (written by analyze_win3.py). Pure reading.
Usage: python -B render_tables.py [raw_dir] > tables.md"""
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W3 = os.path.dirname(HERE)
RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(W3, 'raw')
R = json.load(open(os.path.join(RAW, 'analysis.json'), encoding='utf-8'))
C = R['cells']
ROWS_ORDER = ['D-L4L2', 'J-A', 'D-L5', 'D-L5-npoff', 'D-L5-npon', 'JOLT-T', 'JOLT56-T']
WS = (1, 2, 4, 8, 16)


def f(x, d=3):
    return '--' if x is None else f'{x:.{d}f}'


def cell(row, w):
    return C.get(f'{row}@W{w}')


def yn(b):
    return 'Y' if b else 'n'


out = []
out.append('### Cells: window mean over [0,500), ms/step; median over K processes [min-max]; spread as % of the median\n')
out.append('| row | W | K | median ms | min | max | range % | IQR % | SD % | SE(med) % | per-pass medians | values by process |')
out.append('|---|---|---|---|---|---|---|---|---|---|---|---|')
for row in ROWS_ORDER:
    for w in WS:
        s = cell(row, w)
        if not s:
            continue
        pp = ' / '.join(f'{v:.3f}' for k, v in sorted(s['per_pass_median'].items()))
        out.append(f'| {row} | {w} | {s["n"]} | **{s["median"]:.3f}** | {s["min"]:.3f} | {s["max"]:.3f} | {s["range_pct"]:.2f} | '
                   f'{s["iqr_pct"]:.2f} | {s["sd_pct"]:.2f} | {s["se_med_pct"]:.2f} | {pp} | {", ".join(f"{v:.3f}" for v in s["values_by_process"])} |')

out.append('\n### Sub-windows [0,100) and [100,500), ms/step (median over K [min-max])\n')
out.append('| row | W | [0,100) median | [0,100) min-max | [100,500) median | [100,500) min-max | [100,500) range % | [100,500) SE % |')
out.append('|---|---|---|---|---|---|---|---|')
for row in ROWS_ORDER:
    for w in WS:
        s = cell(row, w)
        if not s:
            continue
        a, b = s['sub'].get('0_100'), s['sub'].get('100_500')
        if not a or not b:
            continue
        out.append(f'| {row} | {w} | {a["median"]:.3f} | {a["min"]:.3f}-{a["max"]:.3f} | {b["median"]:.3f} | {b["min"]:.3f}-{b["max"]:.3f} | {b["range_pct"]:.2f} | {b["se_med_pct"]:.2f} |')

out.append('\n### Jolt\'s own metric: printed steps/s (median over K [min-max]) and 1000/steps_per_s\n')
out.append('| row | W | K | steps/s median | min | max | range % | SE % | ms from steps/s (median) | per-frame mean ms (cell median) | hash | threads |')
out.append('|---|---|---|---|---|---|---|---|---|---|---|---|')
for row in ('JOLT-T', 'JOLT56-T'):
    for w in WS:
        s = cell(row, w)
        if not s or 'steps_per_s' not in s:
            continue
        sp = s['steps_per_s']
        out.append(f'| {row} | {w} | {sp["n"]} | {sp["median"]:.3f} | {sp["min"]:.3f} | {sp["max"]:.3f} | {sp["range_pct"]:.2f} | {sp["se_med_pct"]:.2f} | '
                   f'{s["ms_from_steps_per_s"]["median"]:.4f} | {s["median"]:.4f} | {", ".join(s["hash"])} | {", ".join(str(t) for t in s["threads"])} |')

out.append('\n### boyko receipts per row (per-step CSV columns; the set of distinct per-process values)\n')
out.append('| row | W | manifolds mean [0,100) | [100,500) | [0,500) | final | pairs mean [100,500) | final pairs | pose | expect_pose | pool_workers |')
out.append('|---|---|---|---|---|---|---|---|---|---|---|')
for row in ('D-L4L2', 'J-A', 'D-L5', 'D-L5-npoff', 'D-L5-npon'):
    for w in WS:
        s = cell(row, w)
        if not s or 'manifolds' not in s:
            continue
        m, p = s['manifolds'], s['pairs']
        out.append(f'| {row} | {w} | {m["0_100"]} | {m["100_500"]} | {m["0_500"]} | {m["final"]} | {p["100_500"]} | {p["final"]} | {", ".join(s["pose"])} | {", ".join(str(x) for x in s["expect_pose"])} | {s["pool_workers"]} |')

if R.get('jolt_receipts'):
    out.append('\n### Jolt\'s own receipts (UNTIMED `-receipt` runs, W=1, 500 steps; the patch\'s counting ContactListener)\n')
    out.append('| exe | exit | frames | manifolds mean [0,100) | [100,500) | [0,500) | final | points mean [100,500) | active bodies min | hash |')
    out.append('|---|---|---|---|---|---|---|---|---|---|')
    for r in R['jolt_receipts']:
        m = r.get('manifolds') or {}
        out.append(f'| {r["row"]} | {r["exit"]} | {m.get("n_frames")} | {f(m.get("mean_0_100"), 1)} | {f(m.get("mean_100_500"), 1)} | {f(m.get("mean_0_500"), 1)} | {m.get("final")} | {f(m.get("points_mean_100_500"), 1)} | {m.get("active_bodies_min")} | {r.get("hash")} |')

if R.get('per_manifold'):
    out.append('\n### Per manifold per step over [100,500), each side\'s own count (us)\n')
    out.append('| W | boyko manifolds | boyko us/manifold | Jolt 5.3.0 manifolds | Jolt 5.3.0 us/manifold | ratio b/J5.3 | Jolt 5.6.0 manifolds | Jolt 5.6.0 us/manifold | ratio b/J5.6 |')
    out.append('|---|---|---|---|---|---|---|---|---|')
    for w, e in sorted(R['per_manifold'].items(), key=lambda kv: int(kv[0])):
        out.append(f'| {w} | {f(e.get("boyko_manifolds_100_500"), 1)} | {f(e.get("boyko_us_per_manifold_100_500"), 3)} | '
                   f'{f(e.get("JOLT-T_manifolds_100_500"), 1)} | {f(e.get("JOLT-T_us_per_manifold_100_500"), 3)} | {f(e.get("per_manifold_ratio_boyko_over_JOLT-T"), 3)} | '
                   f'{f(e.get("JOLT56-T_manifolds_100_500"), 1)} | {f(e.get("JOLT56-T_us_per_manifold_100_500"), 3)} | {f(e.get("per_manifold_ratio_boyko_over_JOLT56-T"), 3)} |')

out.append('\n### Comparisons (B against A; effect = median_B/median_A - 1; claimed iff |effect| > 2*hypot(spread_A, spread_B), three readings)\n')
out.append('| comparison | ratio B/A | effect % | bar range % | bar IQR % | bar SE % | claimed range/IQR/SE |')
out.append('|---|---|---|---|---|---|---|')
for k, v in R['comparisons'].items():
    out.append(f'| {k} | {v["ratio"]:.3f} | {v["effect_pct"]:+.2f} | {v["bar_range_pct"]:.2f} | {v["bar_iqr_pct"]:.2f} | {v["bar_se_pct"]:.2f} | {yn(v["claim_range"])}/{yn(v["claim_iqr"])}/{yn(v["claim_se"])} |')

out.append('\n### Bridge to P0b (this window\'s cell against P0b\'s median from MEASUREMENT-QUEUE section 10)\n')
out.append('| cell | P0b median ms | win3 median ms | win3 min-max | shift % | win3 range % | win3 IQR % | win3 SE % | P0b median inside win3 min-max |')
out.append('|---|---|---|---|---|---|---|---|---|')
for k, v in R['bridge_to_p0b'].items():
    out.append(f'| {k} | {v["p0b_median_ms"]:.3f} | {v["win3_median_ms"]:.3f} | {v["win3_min"]:.3f}-{v["win3_max"]:.3f} | {v["shift_pct"]:+.2f} | {v["win3_range_pct"]:.2f} | {v["win3_iqr_pct"]:.2f} | {v["win3_se_pct"]:.2f} | {yn(v["p0b_inside_win3_minmax"])} |')

out.append('\n### Scaling T(1)/T(W)\n')
out.append('| row | W=1 | W=2 | W=4 | W=8 | W=16 |')
out.append('|---|---|---|---|---|---|')
for row, sc in R['scaling'].items():
    out.append(f'| {row} | ' + ' | '.join(f(sc.get(str(w)), 3) for w in WS) + ' |')

rc = R['receipts']
out.append('\n### Receipts and witnesses\n')
out.append(f'- receipts: {rc["n"]} distinct ({rc["n_10s"]} of 10 s); cpu_avg min {rc["min"]} %, median {rc["median"]} %, p90 {rc["p90"]} %, max {rc["max"]} %; '
           f'over 5 %: {len(rc["over5"])}; build processes seen in a receipt: {len(rc["build_procs"])}')
for t, v, names in rc['over5']:
    out.append(f'  - over 5 %: {t} {v} % top3 {names}')
for t, names in rc['build_procs']:
    out.append(f'  - build process in receipt: {t} {names}')
out.append(f'- top process in receipts (count): {rc["top_process_counts"]}')
od = R['others_during']
out.append(f'- other processes\' CPU during a used process: n {od["n"]}, median {od["median"]} %, max {od["max"]} % of the machine; over 5 %: {od["over5"]}')
out.append(f'- contaminated attempts: {len(R["contaminated_attempts"])}; re-runs: {len(R["reruns"])}; excluded slots: {len(R["excluded_slots"])}; invalid attempts: {len(R["invalid_attempts"])}; non-zero exits: {R["nonzero_exits"]}')
for x in R['contaminated_attempts']:
    out.append(f'  - contaminated: {x["row"]}@W{x["W"]} pass {x["pass"]} seq {x["seq"]} ({x["attempt"]}): before {x["rb"]} % after {x["ra"]} %; build before {x["build_before"]} after {x["build_after"]}; top after {[ (t["name"], t["pct_of_machine"]) for t in x["top_after"]]}; mean {f(x["mean_ms"], 4)} ms')
for x in R['reruns']:
    out.append(f'  - re-run: {x["row"]}@W{x["W"]} pass {x["pass"]} seq {x["seq"]} reason {x["reason"]}: valid {x["rerun_valid"]}, contaminated {x["rerun_contaminated"]}, mean {f(x["rerun_mean_ms"], 4)} ms')
for x in R['invalid_attempts']:
    out.append(f'  - invalid: {x}')
out.append(f'- warm-ups (untimed): {R["warmups"]}')
out.append(f'- affinity masks read back: {R["masks"]}; target_env: {R["target_env"]}; profiles: {R["profiles"]}')
out.append(f'- hashes per row: {R["hashes_per_row"]}')
out.append(f'- exe sha256 per row: { {k: [s[:8] for s in v] for k, v in R["exe_sha_per_row"].items()} }')
out.append(f'- void steps non-zero: {R["void_steps_nonzero"]}; drops non-zero: {R["drops_nonzero"]}; disarmed ring traffic non-zero: {R["disarmed_ring_traffic_nonzero"]}')
out.append(f'- Jolt threads (row, W, printed): {R["jolt_threads"]}')
out.append(f'- expect_pose gate (row, W, result): {R["expect_pose_gate"]}')
out.append(f'- % Processor Performance (PDH, machine total, median over processes) by pass: {R["perf_total_by_pass"]}')
wc = R['wall_clock']
out.append('\n### Wall clock\n')
out.append(f'- window start {wc["start"]}, end {wc["end"]}, status `{wc["status"]}`; binaries after: {wc["binaries_after"]}')
out.append(f'- first process {wc["first_process"]}, last process end {wc["last_process_end"]}')
for p, (a, b) in wc['per_pass'].items():
    out.append(f'- pass {p}: {a} -> {b}')
out.append(f'- machine before: {wc["machine_before"]}')
out.append(f'- machine after: {wc["machine_after"]}')
out.append(f'- cells per round: {wc["cells_per_round"]}; pass-0 order: {wc["order_pass0"]}')
print('\n'.join(out))
