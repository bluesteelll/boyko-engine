#!/usr/bin/env python
"""Window 4 (G4 + G5) reduction, recomputed from win4/raw/ by the results-analyst.

Nothing here imports or reads the tester's reduction (raw/g4/g4_reduction.json, raw/g5_reduction.json,
logs/09_*, logs/13_*, window_report.md). Inputs are only:

  raw/runs.jsonl, raw/pass-*/<slot>/run.csv, raw/pass-*/<slot>/pose.bin   (G5)
  raw/g4/runs.jsonl, raw/g4/<group>/<arm>/<param>/<baseline>/estimates.json,
  raw/g4/logs/*.stderr.txt                                                (G4)
  bin/SHA256SUMS
  D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl (Jolt v5.6.0 and D-L5 cells,
                                                                            recomputed, never re-run)

Statistic (the recipe, section 1.1, = the window-3 ruling): a cell is the MEDIAN over K separate
processes; spreads are min-max range, IQR (inclusive/linear quartiles) and the median's SE
(1.2533 * sample SD / sqrt K), each as % of the median; B against A is claimed iff
|median_B / median_A - 1| > 2 * hypot(spread_A, spread_B), printed under every reading.
G4 cells (K=3, or K=6 on the churn row) carry range and SE only (the recipe's "both spreads").

Output: win4/analyst/reduction.json and win4/analyst/tables.txt (printed too).
"""
import csv
import hashlib
import json
import math
import os
import re
import statistics
import sys
from collections import defaultdict, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
W = os.path.dirname(HERE)
RAW = os.path.join(W, "raw")
G4 = os.path.join(RAW, "g4")
OUT = os.path.join(W, "analyst")
WIN3_RUNS = "D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl"
os.makedirs(OUT, exist_ok=True)

LINES = []


def p(*a):
    s = " ".join(str(x) for x in a)
    LINES.append(s)
    print(s)


# ----------------------------------------------------------------------------------------
# statistics
# ----------------------------------------------------------------------------------------
def quantile_linear(v, q):
    """numpy's default (linear) quantile on a sorted list."""
    n = len(v)
    if n == 1:
        return v[0]
    pos = (n - 1) * q
    lo = int(math.floor(pos))
    hi = min(lo + 1, n - 1)
    return v[lo] + (v[hi] - v[lo]) * (pos - lo)


def cell(vals, ids=None):
    v = sorted(vals)
    k = len(v)
    m = statistics.median(v)
    sd = statistics.stdev(v) if k > 1 else 0.0
    q1, q3 = quantile_linear(v, 0.25), quantile_linear(v, 0.75)
    d = OrderedDict(
        k=k,
        median=m,
        min=v[0],
        max=v[-1],
        mean=statistics.fmean(v),
        range_pct=(v[-1] - v[0]) / m * 100 if m else float("nan"),
        iqr_pct=(q3 - q1) / m * 100 if m else float("nan"),
        se_pct=1.2533 * sd / math.sqrt(k) / m * 100 if m else float("nan"),
        values=v,
    )
    if ids is not None:
        d["by_process"] = sorted(zip(vals, ids))
    return d


def compare(A, B, readings=("range", "iqr", "se")):
    """B against A. Effect in % of A; bars = 2*hypot(spread_A, spread_B) per reading."""
    ratio = B["median"] / A["median"]
    eff = (ratio - 1) * 100
    bars = OrderedDict((r, 2 * math.hypot(A[r + "_pct"], B[r + "_pct"])) for r in readings)
    claimed = OrderedDict((r, abs(eff) > bars[r]) for r in readings)
    return OrderedDict(ratio=ratio, effect_pct=eff, delta=B["median"] - A["median"], bars=bars, claimed=claimed)


def fmt_claim(c, readings=("range", "iqr", "se")):
    return "/".join("Y" if c["claimed"][r] else "n" for r in readings)


def fmt_bars(c, readings=("range", "iqr", "se")):
    return " / ".join("%.2f" % c["bars"][r] for r in readings)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# ----------------------------------------------------------------------------------------
# selection (the protocol's rule): per slot the original when valid and uncontaminated, else its re-run
# ----------------------------------------------------------------------------------------
def select(records, slot_key):
    slots = defaultdict(dict)
    for r in records:
        slots[slot_key(r)][r["attempt"]] = r
    used, excluded, why = [], [], {}
    for key in sorted(slots, key=lambda k: tuple(str(x) for x in k)):
        d = slots[key]
        o, rr = d.get("original"), d.get("rerun")

        def ok(x):
            return x is not None and x["exit"] == 0 and x.get("valid", True) and not x.get("contaminated", False)

        if ok(o):
            used.append(o)
            why[key] = "original"
        elif ok(rr):
            used.append(rr)
            why[key] = "rerun (original: %s)" % (
                "invalid " + json.dumps(o.get("invalid")) if not o.get("valid", True) else
                "contaminated before=%s after=%s" % (o.get("contaminated_before"), o.get("contaminated_after")) if o.get("contaminated") else
                "exit %s" % o["exit"])
        else:
            excluded.append(key)
            why[key] = "EXCLUDED (original %s, rerun %s)" % (
                None if o is None else (o["exit"], o.get("valid"), o.get("contaminated")),
                None if rr is None else (rr["exit"], rr.get("valid"), rr.get("contaminated")))
    return used, excluded, why


# ----------------------------------------------------------------------------------------
# G5
# ----------------------------------------------------------------------------------------
def read_csv(path):
    with open(path, newline="", encoding="utf-8") as f:
        rd = csv.DictReader(f)
        rows = list(rd)
    return rows, rd.fieldnames


def g5():
    recs = [json.loads(l) for l in open(os.path.join(RAW, "runs.jsonl"), encoding="utf-8")]
    timed = [r for r in recs if r["attempt"] in ("original", "rerun")]
    warm = [r for r in recs if r["attempt"] == "warmup"]
    used, excluded, why = select(timed, lambda r: (r["pass"], r["seq"]))
    p("== G5 selection: %d records = %d warm-ups + %d originals + %d re-runs; used %d; excluded slots %d" % (
        len(recs), len(warm), sum(1 for r in timed if r["attempt"] == "original"),
        sum(1 for r in timed if r["attempt"] == "rerun"), len(used), len(excluded)))
    for k, v in why.items():
        if v != "original":
            p("   slot pass %d seq %d: %s" % (k[0], k[1], v))

    # --- input checks: recompute the window mean from every used CSV --------------------
    checks = OrderedDict(mean_ms_max_delta=0.0, window_mean_ns_max_delta=0.0, n_steps_not_500=[], nonzero_void=[])
    per = []  # per used process: everything the cells need
    hashes = defaultdict(set)
    pose_files = defaultdict(set)
    for r in used:
        rows, cols = read_csv(r["csv"])
        wall = [int(x["wall_ns"]) for x in rows]
        steps = [int(x["step"]) for x in rows]
        if len(rows) != 500 or steps != list(range(500)):
            checks["n_steps_not_500"].append((r["pass"], r["seq"], len(rows)))
        mean_all = sum(wall) / len(wall) / 1e6
        mean_0_100 = sum(wall[:100]) / 100 / 1e6
        mean_100_500 = sum(wall[100:]) / 400 / 1e6
        checks["mean_ms_max_delta"] = max(checks["mean_ms_max_delta"], abs(mean_all - r["mean_ms"]))
        s = r["summary"]
        checks["window_mean_ns_max_delta"] = max(checks["window_mean_ns_max_delta"], abs(mean_all * 1e6 - s["window_mean_ns"]))
        void_col = sum(int(x["void"]) for x in rows) if "void" in cols else 0
        if void_col or s["void_steps"] != 0 or s["first_void"] is not None:
            checks["nonzero_void"].append((r["pass"], r["seq"], void_col, s["void_steps"]))
        key = (s["scene"], s["cfg"])
        hashes[key].add(s["pose_hash"])
        pose_files[key].add(sha256_file(r["pose"]))
        d = OrderedDict(
            pass_=r["pass"], seq=r["seq"], round_=r["round"], row=r["row"], W=r["W"], attempt=r["attempt"],
            slot=os.path.basename(r["cwd"]), start=r["start"], wall_s=r["wall_s"],
            mean_all=mean_all, mean_0_100=mean_0_100, mean_100_500=mean_100_500,
            median_step_100_500=statistics.median(wall[100:]) / 1e6,
            pose=s["pose_hash"], expect_pose=s["expect_pose"], scene=s["scene"], cfg=s["cfg"],
            bodies=s["bodies"], workers=s["workers"], pool_workers=s["threads"]["pool_workers"],
            dispatcher=s["threads"]["dispatcher"], armed=s["armed"], canary_ns=s["canary_ns"],
            broadphase=s["config"]["broadphase"], select=s["config"]["broadphase_select"],
            brute_max_rows=s["config"]["tree_brute_max_rows"],
            simd_solve=s["config"]["simd_solve"], parallel_solve=s["config"]["parallel_solve"],
            parallel_np=s["config"]["parallel_narrowphase"], sleeping=s["config"]["sleeping"],
            final_manifolds=s["final_manifolds"], final_pairs=s["final_pairs"], final_top_y=s["final_top_y"],
            diag=s["broadphase_tree"], target_env=s["target_env"], debug_assertions=s["debug_assertions"],
            zone_tier=s["profile_name"], mask=r["mask_readback"], exe_sha256=r["exe_sha256"],
            rb=r["receipt_before"]["cpu_avg"], ra=r["receipt_after"]["cpu_avg"],
            build_before=r["receipt_before"].get("build_procs_busy", []), build_after=r["receipt_after"].get("build_procs_busy", []),
            others_busy_pct=r["others_busy_pct"], others_top5=r["others_top5"], waited_s=r["waited_s"],
            manifolds_100_500=statistics.fmean(int(x["manifolds"]) for x in rows[100:]),
            pairs_100_500=statistics.fmean(int(x["pairs"]) for x in rows[100:]),
        )
        # armed rows: the four tree spans, the broadphase system span, the structure receipts
        if s["armed"] or "sys_parity_canary_ns" in cols:
            def col(name):
                return [int(x[name]) for x in rows]
            spans = OrderedDict()
            for nm in ("phys_bp_verify", "phys_bp_build", "phys_bp_query", "phys_bp_assemble"):
                v = col(nm + "_ns")
                n = col(nm + "_n")
                spans[nm] = OrderedDict(
                    median_100_500_ms=statistics.median(v[100:]) / 1e6,
                    mean_0_500_ms=sum(v) / 500 / 1e6,
                    n_values=sorted(set(n)),
                )
            sigma = [sum(int(x[nm + "_ns"]) for nm in ("phys_bp_verify", "phys_bp_build", "phys_bp_query", "phys_bp_assemble")) for x in rows]
            spans["sigma"] = OrderedDict(median_100_500_ms=statistics.median(sigma[100:]) / 1e6, mean_0_500_ms=sum(sigma) / 500 / 1e6,
                                         max_ms=max(sigma) / 1e6, min_100_500_ms=min(sigma[100:]) / 1e6)
            bp = col("sys_physics_broadphase_ns")
            spans["sys_physics_broadphase"] = OrderedDict(median_100_500_ms=statistics.median(bp[100:]) / 1e6, mean_0_500_ms=sum(bp) / 500 / 1e6,
                                                          n_values=sorted(set(col("sys_physics_broadphase_n"))))
            sel = col("sys_select_broadphase_ns")
            spans["sys_select_broadphase"] = OrderedDict(median_100_500_ms=statistics.median(sel[100:]) / 1e6, mean_0_500_ms=sum(sel) / 500 / 1e6)
            q, mbr, reb = col("phys_bp_queried"), col("phys_bp_members"), col("phys_bp_rebuilds")
            spans["structure"] = OrderedDict(
                queried_plus_members=sorted(set(a + b for a, b in zip(q, mbr))),
                queried=sorted(set(q)), members=sorted(set(mbr)), rebuilds_total=sum(reb),
                bp_pairs_final=int(rows[-1]["phys_bp_pairs"]), np_pairs_final=int(rows[-1]["phys_np_pairs"]),
                span_n_sets=sorted(set((int(x["phys_bp_verify_n"]), int(x["phys_bp_build_n"]), int(x["phys_bp_query_n"]), int(x["phys_bp_assemble_n"])) for x in rows)),
            )
            if "sys_parity_canary_ns" in cols:
                cn = col("sys_parity_canary_ns")
                spans["sys_parity_canary"] = OrderedDict(median_100_500_ms=statistics.median(cn[100:]) / 1e6, mean_0_500_ms=sum(cn) / 500 / 1e6,
                                                         n_values=sorted(set(col("sys_parity_canary_n"))))
            d["spans"] = spans
        per.append(d)

    p("== G5 input checks: max |csv mean - driver mean_ms| = %.3g ms; max |csv mean - runner window_mean_ns| = %.3g ns; steps!=500: %s; void: %s" % (
        checks["mean_ms_max_delta"], checks["window_mean_ns_max_delta"], checks["n_steps_not_500"], checks["nonzero_void"]))
    p("== G5 pose hashes per (scene, cfg): %s" % {"/".join(k): sorted(v) for k, v in hashes.items()})
    p("== G5 pose.bin sha256 per (scene, cfg): %s" % {"/".join(k): [x[:12] for x in sorted(v)] for k, v in pose_files.items()})
    from collections import Counter
    p("== G5 expect_pose: %s" % Counter(d["expect_pose"] for d in per))
    p("== G5 knobs: %s" % Counter((d["row"], d["broadphase"], d["select"], d["brute_max_rows"], d["simd_solve"], d["parallel_solve"], d["parallel_np"], d["sleeping"]) for d in per))
    p("== G5 threads: pool_workers==W: %s; dispatcher: %s; mask: %s; target_env: %s; debug_assertions: %s; zone tier: %s; exe sha: %s" % (
        all(d["pool_workers"] == d["W"] for d in per), Counter(d["dispatcher"] for d in per), Counter(d["mask"] for d in per),
        Counter(d["target_env"] for d in per), Counter(d["debug_assertions"] for d in per), Counter(d["zone_tier"] for d in per),
        Counter(d["exe_sha256"][:8] for d in per)))
    p("== G5 counts: final manifolds/pairs per scene: %s" % Counter((d["scene"], d["final_manifolds"], d["final_pairs"]) for d in per))
    # structural gate (2.2 item 2) per process
    bad_diag = []
    for d in per:
        g = d["diag"]
        if d["broadphase"] == "Tree" and d["scene"] in ("jolt", "rest"):
            exp = dict(static_rebuilds=1, members=1, evictions=0, translations=0, wide_rows=0, excluded_rows=0, sleeper_rebuilds=0, hint_candidates=0, patches=0, locator_resets=0)
        else:
            exp = {k: 0 for k in g}
        if any(g[k] != v for k, v in exp.items()):
            bad_diag.append((d["slot"], g))
    p("== G5 TreeDiag gate (2.2 item 2): violations %s" % (bad_diag or "none"))
    inv = [r for r in timed if not r.get("valid", True)]
    for r in inv:
        twin = [x for x in used if x["row"] == r["row"] and x["W"] == r["W"]][0]
        a, b = open(r["pose"], "rb").read(), open(twin["pose"], "rb").read()
        nd = sum(1 for x, y in zip(a, b) if x != y)
        ra_, rb_ = read_csv(r["csv"])[0], read_csv(twin["csv"])[0]
        first = next((i for i, (x, y) in enumerate(zip(ra_, rb_)) if (x["manifolds"], x["pairs"], x["top_y"]) != (y["manifolds"], y["pairs"], y["top_y"])), None)
        ndiff = sum(1 for x, y in zip(ra_, rb_) if x["top_y"] != y["top_y"]), sum(1 for x, y in zip(ra_, rb_) if x["manifolds"] != y["manifolds"]), sum(1 for x, y in zip(ra_, rb_) if x["pairs"] != y["pairs"])
        checks["invalid_process"] = OrderedDict(slot=os.path.basename(r["cwd"]), invalid=r["invalid"], pose=r["summary"]["pose_hash"], pose_bytes_differ=nd, of=len(a), twin=os.path.basename(twin["cwd"]),
                                                first_differing_step=first, steps_differing_top_y_manifolds_pairs=ndiff, at_first=(ra_[first]["top_y"], rb_[first]["top_y"]) if first is not None else None,
                                                knobs=r["summary"]["config"], threads=r["summary"]["threads"], others_busy_pct=r["others_busy_pct"], receipts=(r["receipt_before"]["cpu_avg"], r["receipt_after"]["cpu_avg"]), mean_ms=r["mean_ms"])
        p("== G5 invalid process %s: %s; pose bytes differing from %s: %d of %d; first differing step %s (top_y %s); steps differing (top_y, manifolds, pairs) %s; others %.2f %%; receipts %s" % (
            checks["invalid_process"]["slot"], r["invalid"], checks["invalid_process"]["twin"], nd, len(a), first, checks["invalid_process"]["at_first"], ndiff, r["others_busy_pct"], checks["invalid_process"]["receipts"]))

    # receipts
    rcpts = []
    for r in timed:
        rcpts.append(r["receipt_before"]["cpu_avg"])
        rcpts.append(r["receipt_after"]["cpu_avg"])
    rc = sorted(rcpts)
    build_seen = [r for r in timed if r["receipt_before"].get("build_procs_busy") or r["receipt_after"].get("build_procs_busy")]
    others = sorted(d["others_busy_pct"] for d in per)
    p("== G5 receipts (all timed attempts, before+after): n=%d median %.2f p90 %.2f max %.2f; >5%%: %d; build procs seen: %d; contaminated attempts: %d" % (
        len(rc), statistics.median(rc), quantile_linear(rc, 0.9), rc[-1], sum(1 for x in rc if x > 5), len(build_seen),
        sum(1 for r in timed if r["contaminated"])))
    p("== G5 during-process witness others_busy_pct over used: median %.2f p95 %.2f max %.2f; >3%%: %s; waited_s nonzero: %d" % (
        statistics.median(others), quantile_linear(others, 0.95), others[-1],
        [(d["slot"], d["others_busy_pct"], d["others_top5"][:2]) for d in per if d["others_busy_pct"] > 3],
        sum(1 for d in per if d["waited_s"])))

    # --- cells --------------------------------------------------------------------------
    cells = OrderedDict()
    groups = defaultdict(list)
    for d in per:
        groups[(d["row"], d["W"])].append(d)
    for key in sorted(groups, key=lambda k: (k[1], k[0])):
        g = groups[key]
        c = cell([d["mean_all"] for d in g], [d["slot"] for d in g])
        c["sub_0_100"] = statistics.median(d["mean_0_100"] for d in g)
        c["sub_100_500"] = statistics.median(d["mean_100_500"] for d in g)
        c["manifolds_100_500"] = statistics.median(d["manifolds_100_500"] for d in g)
        c["by_pass"] = {ps: sorted(d["mean_all"] for d in g if d["pass_"] == ps) for ps in (0, 1)}
        cells["%s@W%d" % key] = c
    p("\n== G5 cells (median over K of the [0,500) window mean, ms/step; range / IQR / SE %; values)")
    for k, c in cells.items():
        p("  %-24s K=%d  %9.4f [%.4f-%.4f]  %5.2f / %5.2f / %5.2f   sub [0,100)/[100,500) %.4f / %.4f   %s" % (
            k, c["k"], c["median"], c["min"], c["max"], c["range_pct"], c["iqr_pct"], c["se_pct"], c["sub_0_100"], c["sub_100_500"],
            ", ".join("%.4f" % v for v in c["values"])))

    # --- window 3 cells recomputed (Jolt v5.6.0, D-L5) ----------------------------------
    w3 = [json.loads(l) for l in open(WIN3_RUNS, encoding="utf-8")]
    w3t = [r for r in w3 if r["attempt"] in ("original", "rerun")]
    w3used, w3exc, _ = select(w3t, lambda r: (r["pass"], r["seq"]))
    w3cells = OrderedDict()
    for row in ("JOLT56-T", "D-L5"):
        for Wn in (1, 2, 4, 8, 16):
            g = [r for r in w3used if r["row"] == row and r["W"] == Wn]
            if g:
                w3cells["%s@W%d" % (row, Wn)] = cell([r["mean_ms"] for r in g], [os.path.basename(r["cwd"]) for r in g])
    p("\n== window 3 cells recomputed from its runs.jsonl (%d used of %d timed; excluded slots %d)" % (len(w3used), len(w3t), len(w3exc)))
    for k, c in w3cells.items():
        p("  %-14s K=%d %9.4f [%.4f-%.4f] %5.2f / %5.2f / %5.2f  %s" % (k, c["k"], c["median"], c["min"], c["max"], c["range_pct"], c["iqr_pct"], c["se_pct"], ", ".join("%.3f" % v for v in c["values"])))

    # --- comparisons --------------------------------------------------------------------
    comps = OrderedDict()

    def cmp(name, A, B):
        c = compare(A, B)
        comps[name] = c
        p("  %-52s %9.4f -> %9.4f  ratio %.4f  %+7.2f %%  (%+.4f ms)  bars %s  %s" % (name, A["median"], B["median"], c["ratio"], c["effect_pct"], c["delta"], fmt_bars(c), fmt_claim(c)))
        return c

    p("\n== G5 comparisons: tree against allpairs (B against A; effect = B/A - 1; bars r / i / s %; claimed r/i/s)")
    for Wn in (1, 2, 4, 8, 16):
        cmp("T-A-tree vs T-A-allpairs @W%d" % Wn, cells["T-A-allpairs@W%d" % Wn], cells["T-A-tree@W%d" % Wn])
    for Wn in (1, 8):
        cmp("T-D-tree vs T-D-allpairs @W%d" % Wn, cells["T-D-allpairs@W%d" % Wn], cells["T-D-tree@W%d" % Wn])
        cmp("R-tree vs R-allpairs @W%d" % Wn, cells["R-allpairs@W%d" % Wn], cells["R-tree@W%d" % Wn])
        cmp("T-A-tree-armed vs T-A-allpairs-armed @W%d" % Wn, cells["T-A-allpairs-armed@W%d" % Wn], cells["T-A-tree-armed@W%d" % Wn])
        cmp("T-A-tree-armed vs T-A-tree (arming cost) @W%d" % Wn, cells["T-A-tree@W%d" % Wn], cells["T-A-tree-armed@W%d" % Wn])
        cmp("T-A-allpairs-armed vs T-A-allpairs (arming) @W%d" % Wn, cells["T-A-allpairs@W%d" % Wn], cells["T-A-allpairs-armed@W%d" % Wn])
        cmp("T-C-tree vs T-A-tree (canary rise) @W%d" % Wn, cells["T-A-tree@W%d" % Wn], cells["T-C-tree@W%d" % Wn])
    cmp("S16-tree vs S16-allpairs @W1", cells["S16-allpairs@W1"], cells["S16-tree@W1"])
    p("\n== the bridge: T-D-allpairs (this window) against window 3's D-L5 (same physics code, de06b6c9 = a46b8287 minus the C3 tree)")
    for Wn in (1, 8):
        cmp("T-D-allpairs@W%d vs win3 D-L5@W%d" % (Wn, Wn), w3cells["D-L5@W%d" % Wn], cells["T-D-allpairs@W%d" % Wn])
        cmp("T-D-tree@W%d vs win3 D-L5@W%d" % (Wn, Wn), w3cells["D-L5@W%d" % Wn], cells["T-D-tree@W%d" % Wn])
    p("\n== against Jolt v5.6.0 (window 3 cells; boyko / Jolt)")
    for Wn in (1, 8):
        cmp("T-D-tree@W%d / JOLT56-T@W%d" % (Wn, Wn), w3cells["JOLT56-T@W%d" % Wn], cells["T-D-tree@W%d" % Wn])
        cmp("T-D-allpairs@W%d / JOLT56-T@W%d" % (Wn, Wn), w3cells["JOLT56-T@W%d" % Wn], cells["T-D-allpairs@W%d" % Wn])
    for Wn in (1, 2, 4, 8, 16):
        cmp("T-A-tree@W%d / JOLT56-T@W%d" % (Wn, Wn), w3cells["JOLT56-T@W%d" % Wn], cells["T-A-tree@W%d" % Wn])
    p("\n== scaling")
    for row in ("T-A-tree", "T-A-allpairs"):
        t1 = cells[row + "@W1"]["median"]
        p("  %s T(1)/T(W): %s ; T(8)/T(16) = %.3f" % (row, ", ".join("W%d %.3f" % (Wn, t1 / cells[row + "@W%d" % Wn]["median"]) for Wn in (2, 4, 8, 16)),
                                                     cells[row + "@W8"]["median"] / cells[row + "@W16"]["median"]))
        cmp("%s W16 vs W8" % row, cells[row + "@W8"], cells[row + "@W16"])
    for row in ("T-D-tree", "T-D-allpairs", "R-tree", "R-allpairs"):
        p("  %s T(1)/T(8) = %.3f" % (row, cells[row + "@W1"]["median"] / cells[row + "@W8"]["median"]))
    p("  JOLT56-T T(1)/T(8) = %.3f" % (w3cells["JOLT56-T@W1"]["median"] / w3cells["JOLT56-T@W8"]["median"]))

    # --- armed rows: spans, delta-bp, t_q, structure ------------------------------------
    p("\n== armed rows: per-process median over steps [100,500) of each span, then the cell median over K (ms)")
    armed = OrderedDict()
    for key in ("T-A-tree-armed@W1", "T-A-tree-armed@W8", "T-A-allpairs-armed@W1", "T-A-allpairs-armed@W8", "T-C-tree@W1", "T-C-tree@W8"):
        row, Wn = key.split("@W")
        g = [d for d in per if d["row"] == row and d["W"] == int(Wn)]
        a = OrderedDict()
        for nm in ("phys_bp_verify", "phys_bp_build", "phys_bp_query", "phys_bp_assemble", "sigma", "sys_physics_broadphase", "sys_select_broadphase", "sys_parity_canary"):
            if nm in g[0]["spans"]:
                a[nm] = OrderedDict(
                    median_100_500=cell([d["spans"][nm]["median_100_500_ms"] for d in g]),
                    mean_0_500=cell([d["spans"][nm]["mean_0_500_ms"] for d in g]),
                )
        a["structure"] = OrderedDict(
            span_n_sets=sorted(set(tuple(x) for d in g for x in d["spans"]["structure"]["span_n_sets"])),
            queried_plus_members=sorted(set(x for d in g for x in d["spans"]["structure"]["queried_plus_members"])),
            queried=sorted(set(x for d in g for x in d["spans"]["structure"]["queried"])),
            members=sorted(set(x for d in g for x in d["spans"]["structure"]["members"])),
            rebuilds_total=sorted(set(d["spans"]["structure"]["rebuilds_total"] for d in g)),
            bp_pairs_final=sorted(set(d["spans"]["structure"]["bp_pairs_final"] for d in g)),
            sigma_max_ms=max(d["spans"]["sigma"]["max_ms"] for d in g),
            canary_ns=sorted(set(d["canary_ns"] for d in g)),
            armed=sorted(set(d["armed"] for d in g)),
        )
        armed[key] = a
        p("  %s: armed=%s structure %s" % (key, a["structure"]["armed"], {k: v for k, v in a["structure"].items() if k != "armed"}))
        for nm, v in a.items():
            if nm == "structure":
                continue
            p("     %-24s median[100,500): %.4f [%.4f-%.4f] (range %.2f%%, SE %.2f%%)   mean[0,500): %.4f [%.4f-%.4f]   values %s" % (
                nm, v["median_100_500"]["median"], v["median_100_500"]["min"], v["median_100_500"]["max"], v["median_100_500"]["range_pct"], v["median_100_500"]["se_pct"],
                v["mean_0_500"]["median"], v["mean_0_500"]["min"], v["mean_0_500"]["max"], ", ".join("%.4f" % x for x in v["median_100_500"]["values"])))

    p("\n== the tree span (gate 2.3): sigma of the four phys_bp spans, median over [100,500) then over K; limit 0.36 (W1) / 0.35 (W8) ms")
    gates = OrderedDict()
    for Wn, lim in ((1, 0.36), (8, 0.35)):
        s = armed["T-A-tree-armed@W%d" % Wn]["sigma"]["median_100_500"]["median"]
        gates["tree_span_W%d" % Wn] = OrderedDict(value=s, limit=lim, pass_=s <= lim)
        p("  W%d: sigma %.4f ms vs %.2f -> %s (verify %.4f build %.4f query %.4f assemble %.4f)" % (
            Wn, s, lim, "PASS" if s <= lim else "FAIL",
            *[armed["T-A-tree-armed@W%d" % Wn][nm]["median_100_500"]["median"] for nm in ("phys_bp_verify", "phys_bp_build", "phys_bp_query", "phys_bp_assemble")]))

    p("\n== delta-bp (gate 2.3): physics_broadphase system span, allpairs-armed minus tree-armed; bar 1.03 ms at W=8")
    for Wn in (1, 8):
        A = armed["T-A-allpairs-armed@W%d" % Wn]["sys_physics_broadphase"]
        B = armed["T-A-tree-armed@W%d" % Wn]["sys_physics_broadphase"]
        for stat in ("median_100_500", "mean_0_500"):
            c = compare(A[stat], B[stat])
            dl = A[stat]["median"] - B[stat]["median"]
            gates["delta_bp_W%d_%s" % (Wn, stat)] = OrderedDict(allpairs=A[stat]["median"], tree=B[stat]["median"], delta=dl, bar=1.03 if Wn == 8 else None,
                                                                pass_=(dl >= 1.03) if Wn == 8 else None, cmp=c)
            p("  W%d %-14s allpairs %.4f [%.4f-%.4f] tree %.4f [%.4f-%.4f]  delta %.4f ms  tree/allpairs %.4f (%+.1f %%) bars %s %s" % (
                Wn, stat, A[stat]["median"], A[stat]["min"], A[stat]["max"], B[stat]["median"], B[stat]["min"], B[stat]["max"], dl, c["ratio"], c["effect_pct"], fmt_bars(c), fmt_claim(c)))
        # the design's "bp before" (P0b): 2.100 at W1, 1.945 at W8
        p("     P0b's armed bp (design table): %.3f ms; tree-armed step vs allpairs-armed step delta %.4f ms" % (
            2.100 if Wn == 1 else 1.945, cells["T-A-allpairs-armed@W%d" % Wn]["median"] - cells["T-A-tree-armed@W%d" % Wn]["median"]))

    p("\n== t_q (1.3): T-A-tree-armed@W1 phys_bp_query_ns, median over [100,500) then over K")
    tq = armed["T-A-tree-armed@W1"]["phys_bp_query"]["median_100_500"]
    tq8 = armed["T-A-tree-armed@W8"]["phys_bp_query"]["median_100_500"]
    p("  t_q(J, W1) = %.4f ms [%.4f-%.4f]; per row %.1f ns; at W8 %.4f ms" % (tq["median"], tq["min"], tq["max"], tq["median"] * 1e6 / 1240, tq8["median"]))
    d6 = OrderedDict(t_q_ms=tq["median"], per_row_ns=tq["median"] * 1e6 / 1240)
    for row in ("T-A-tree", "T-D-tree"):
        T8 = cells[row + "@W8"]["median"]
        five = 0.05 * T8
        best = tq["median"] * (1 - 1 / 8)  # E = 1, omega = 0
        d6[row] = OrderedDict(T8=T8, five_pct=five, best_case_saving_E1_omega0=best,
                              omega_budget_E1=best - five, saving_E05=tq["median"] * (1 - 1 / 4), omega_budget_E05=tq["median"] * 0.75 - five,
                              min_E_for_trigger_omega0=(1 / (8 * (1 - five / tq["median"]))) if five < tq["median"] else None)
        p("  D6 on %s: T(8) %.4f, 5 %% = %.4f ms; t_q*(1-1/8) at E=1, omega=0: %.4f (omega budget %.4f); at E=0.5: %.4f (budget %.4f); E needed at omega=0: %s" % (
            row, T8, five, best, best - five, tq["median"] * 0.75, tq["median"] * 0.75 - five,
            "%.3f" % d6[row]["min_E_for_trigger_omega0"] if d6[row]["min_E_for_trigger_omega0"] else "n/a"))
    p("  design D6 arithmetic: saving 0.095-0.145 ms = 1.0-1.6 %% of T(8)=9.162 (deferred); design c_q 100-150 ns/row => t_q 0.124-0.186 ms")

    p("\n== canary (gate 2.3): T-C-tree; the canary's own system span vs canary_ns; the step rise vs T-A-tree")
    for Wn in (1, 8):
        a = armed["T-C-tree@W%d" % Wn]
        cn = a["structure"]["canary_ns"]
        span = a["sys_parity_canary"]["median_100_500"]
        rise = comps["T-C-tree vs T-A-tree (canary rise) @W%d" % Wn]
        gates["canary_W%d" % Wn] = OrderedDict(canary_ns=cn, span_median_ms=span["median"], span_over_canary=[span["median"] * 1e6 / c for c in cn], rise_ms=rise["delta"], rise_over_canary=[rise["delta"] * 1e6 / c for c in cn], rise_cmp=rise)
        p("  W%d: canary_ns %s (%.4f ms); span median[100,500) %.4f [%.4f-%.4f] ms (span/canary %s); step rise %+.4f ms (%s of canary) %s bars %s" % (
            Wn, cn, cn[0] / 1e6, span["median"], span["min"], span["max"], ["%.4f" % x for x in gates["canary_W%d" % Wn]["span_over_canary"]],
            rise["delta"], ["%.3f" % x for x in gates["canary_W%d" % Wn]["rise_over_canary"]], fmt_claim(rise), fmt_bars(rise)))

    return OrderedDict(selection=OrderedDict(used=len(used), excluded=[list(k) for k in excluded], why={"%d/%d" % k: v for k, v in why.items() if v != "original"}),
                       checks=checks, hashes={"/".join(k): sorted(v) for k, v in hashes.items()}, pose_files={"/".join(k): sorted(v) for k, v in pose_files.items()},
                       bad_diag=bad_diag, cells=cells, win3=w3cells, comparisons=comps, armed=armed, gates=gates, d6=d6, per_process=per)


# ----------------------------------------------------------------------------------------
# G4
# ----------------------------------------------------------------------------------------
G4_SIZES = [17, 64, 128, 256, 1000, 10000, 100000]
ARMS = ["all_pairs", "grid_w1", "grid_w8", "tree"]
MAINT = ["stable", "admission_from_empty", "admission_of_64", "eviction_filter", "shift_translation", "compaction", "high_jumper"]
MS = [1240, 10000, 100000]
CHURN_ARMS = ["stable", "swap_churn", "archetype_shift", "first_archetype_spawn", "burst_despawn", "burst_migrate"]

# the recipe's dry-run structural receipts (section 1.1)
RECIPE_RECEIPTS = {
    "bp_g4_scene/1240": dict(rows=1241, pairs=9570, members=1, static_rebuilds=1),
    "bp_g4_scene/j100": dict(rows=1241, pairs=9564, members=1, static_rebuilds=1),
    "bp_g4_scene/10000": dict(pairs=83559, members=1),
    "bp_g4_scene/100000": dict(pairs=840494, members=1),
    "bp_g4_uniform/100000": dict(pairs=293501, members=0),
    "bp_g4_disparity/100000": dict(pairs=574763, members=0),
}
RECIPE_STABLE_PAIRS = {1240: 9464, 10000: 83149, 100000: 867834}


def parse_receipts(path):
    out = OrderedDict()
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.strip()
        m = re.match(r"^(bp_g4_\w+)/(\d+|j100): rows (\d+) pairs (\d+) tree members (\d+) static_rebuilds (\d+) wide (\d+) excluded (\d+)", line)
        if m:
            out["%s/%s" % (m.group(1), m.group(2))] = dict(rows=int(m.group(3)), pairs=int(m.group(4)), members=int(m.group(5)), static_rebuilds=int(m.group(6)), wide=int(m.group(7)), excluded=int(m.group(8)))
            continue
        m = re.match(r"^bp_g4_maintenance/(\w+)/(\d+): timed step: evictions \+(\d+) translations \+(\d+) patches \+(\d+) static_rebuilds \+(\d+); members (\d+); pairs (\d+) \((.*)\)", line)
        if m:
            out["bp_g4_maintenance/%s/%s" % (m.group(1), m.group(2))] = dict(evictions=int(m.group(3)), translations=int(m.group(4)), patches=int(m.group(5)), static_rebuilds=int(m.group(6)), members=int(m.group(7)), pairs=int(m.group(8)), oracle=m.group(9))
            continue
        m = re.match(r"^bp_g4_maintenance/high_jumper/(\d+): the mover \(row (\d+)\) has (\d+) higher-row partners", line)
        if m:
            out["bp_g4_maintenance/high_jumper/%s/mover" % m.group(1)] = dict(row=int(m.group(2)), partners=int(m.group(3)))
            continue
        m = re.match(r"^row_identity_churn/(\w+)/(\w+): tree over the receipt: translations \+(\d+) evictions \+(\d+) patches \+(\d+); static_rebuilds (\d+) members (\d+)", line)
        if m:
            out["row_identity_churn/%s/%s/tree" % (m.group(1), m.group(2))] = dict(translations=int(m.group(3)), evictions=int(m.group(4)), patches=int(m.group(5)), static_rebuilds=int(m.group(6)), members=int(m.group(7)))
            continue
        m = re.match(r"^row_identity_churn/(\w+)/(\w+): 4 churn steps built (\d+) maps and resolved (\d+) rows by stage 2", line)
        if m:
            out["row_identity_churn/%s/%s/maps" % (m.group(1), m.group(2))] = dict(maps=int(m.group(3)), resolved=int(m.group(4)))
    return out


def g4(tq_ms):
    recs = [json.loads(l) for l in open(os.path.join(G4, "runs.jsonl"), encoding="utf-8")]
    used, excluded, why = select(recs, lambda r: (r["block"], r["k"], r["seq"], r["filter_root"]))
    p("\n\n== G4 selection: %d records (%d originals + %d re-runs); used %d; excluded %d" % (
        len(recs), sum(1 for r in recs if r["attempt"] == "original"), sum(1 for r in recs if r["attempt"] == "rerun"), len(used), len(excluded)))
    for k, v in why.items():
        if v != "original":
            p("   %s k%d seq %d %s: %s" % (k[0], k[1], k[2], k[3], v))
    exes = set((r["exe"], r["exe_sha256"]) for r in used)
    p("== G4 exes: %s" % sorted((os.path.basename(e), s[:12]) for e, s in exes))
    p("== G4 build procs during / voided: %s / %s; exit codes %s; masks %s" % (
        sum(1 for r in recs if r["build_procs_during"]), sum(1 for r in recs if r["voided_during"]), sorted(set(r["exit"] for r in recs)), sorted(set(r["mask_readback"] for r in recs))))
    rc = sorted(x for r in recs for x in (r["receipt_before"]["cpu_avg"], r["receipt_after"]["cpu_avg"]))
    others = sorted(r["others_busy_pct"] for r in used)
    p("== G4 receipts (all attempts): n=%d median %.2f p90 %.2f max %.2f; >5%%: %d; contaminated attempts %d (before %d / after %d); others_busy_pct over used: median %.2f p95 %.2f max %.2f; waited_s>0: %d" % (
        len(rc), statistics.median(rc), quantile_linear(rc, 0.9), rc[-1], sum(1 for x in rc if x > 5), sum(1 for r in recs if r["contaminated"]),
        sum(1 for r in recs if r["contaminated_before"]), sum(1 for r in recs if r["contaminated_after"]),
        statistics.median(others), quantile_linear(others, 0.95), others[-1], sum(1 for r in used if r["waited_s"])))
    p("== G4 timing: first start %s, last end %s" % (min(r["start"] for r in recs), max(r["end"] for r in recs)))

    # --- per-cell values from estimates.json ------------------------------------------
    vals = defaultdict(list)  # cell id -> [(median_ns, baseline, wall)]
    receipts = defaultdict(list)  # receipt key -> [(values, source)]
    missing = []
    for r in used:
        b = r["baseline"]
        fr = r["filter_root"]
        stem = "%03d_%s_k%d_%s%s" % (r["seq"], r["block"], r["k"], fr.replace("/", "_"), "_rerun" if r["attempt"] == "rerun" else "")
        errf = os.path.join(G4, "logs", stem + ".stderr.txt")
        for key, v in parse_receipts(errf).items():
            receipts[key].append((json.dumps(v, sort_keys=True), stem))
        if fr.startswith("bp_g4_"):
            grp = fr
            arms = ARMS if grp != "bp_g4_maintenance" else MAINT
            params = (G4_SIZES + (["j100"] if grp == "bp_g4_scene" else [])) if grp != "bp_g4_maintenance" else MS
            if grp == "bp_g4_scene":
                params = [1240, 10000, 100000, "j100"]
            for arm in arms:
                for prm in params:
                    f = os.path.join(G4, grp, arm, str(prm), b, "estimates.json")
                    if not os.path.exists(f):
                        missing.append(f)
                        continue
                    e = json.load(open(f, encoding="utf-8"))
                    vals["%s/%s/%s" % (grp, arm, prm)].append((e["median"]["point_estimate"], b, e["median"]["standard_error"]))
        else:
            _, arm, label = fr.split("/")
            f = os.path.join(G4, "row_identity_churn", arm, label, b, "estimates.json")
            if not os.path.exists(f):
                missing.append(f)
                continue
            e = json.load(open(f, encoding="utf-8"))
            vals[fr].append((e["median"]["point_estimate"], b, e["median"]["standard_error"]))
    p("== G4 estimates.json files read: %d cells; missing files: %s" % (len(vals), missing or "none"))

    # structural receipts: identical across K, and equal to the recipe's dry run
    diffs = {k: set(v for v, _ in lst) for k, lst in receipts.items()}
    non_identical = {k: v for k, v in diffs.items() if len(v) > 1}
    p("== G4 structural receipts: %d keys; non-identical across processes: %s" % (len(diffs), non_identical or "none"))
    rec_check = []
    for key, exp in RECIPE_RECEIPTS.items():
        got = json.loads(sorted(diffs[key])[0]) if key in diffs else None
        ok = got is not None and all(got.get(k) == v for k, v in exp.items())
        rec_check.append((key, exp, {k: got.get(k) for k in exp} if got else None, ok))
    for m in MS:
        got = json.loads(sorted(diffs["bp_g4_maintenance/stable/%d" % m])[0])
        rec_check.append(("stable/%d pairs" % m, RECIPE_STABLE_PAIRS[m], got["pairs"], got["pairs"] == RECIPE_STABLE_PAIRS[m]))
        hj = json.loads(sorted(diffs["bp_g4_maintenance/high_jumper/%d" % m])[0])
        mv = json.loads(sorted(diffs["bp_g4_maintenance/high_jumper/%d/mover" % m])[0])
        rec_check.append(("high_jumper/%d" % m, "partners 9, translations +1, patches +18, evictions 0, rebuilds 0", (mv["partners"], hj["translations"], hj["patches"], hj["evictions"], hj["static_rebuilds"]),
                          (mv["partners"], hj["translations"], hj["patches"], hj["evictions"], hj["static_rebuilds"]) == (9, 1, 18, 0, 0)))
        cp = json.loads(sorted(diffs["bp_g4_maintenance/compaction/%d" % m])[0])
        rec_check.append(("compaction/%d" % m, "evictions +1, rebuilds +1, members m/2", (cp["evictions"], cp["static_rebuilds"], cp["members"]), (cp["evictions"], cp["static_rebuilds"], cp["members"]) == (1, 1, m // 2)))
        a64 = json.loads(sorted(diffs["bp_g4_maintenance/admission_of_64/%d" % m])[0])
        rec_check.append(("admission_of_64/%d" % m, "rebuilds +1, members m+64", (a64["static_rebuilds"], a64["members"]), (a64["static_rebuilds"], a64["members"]) == (1, m + 64)))
        st = json.loads(sorted(diffs["bp_g4_maintenance/shift_translation/%d" % m])[0])
        rec_check.append(("shift_translation/%d" % m, "translations +1, patches 0, evictions 0", (st["translations"], st["patches"], st["evictions"]), (st["translations"], st["patches"], st["evictions"]) == (1, 0, 0)))
    oracle = sorted(set(json.loads(v)["oracle"] for k, s in diffs.items() if k.startswith("bp_g4_maintenance/") and not k.endswith("/mover") for v in s))
    rec_check.append(("maintenance oracle", "oracle ok", oracle, oracle == ["oracle ok"]))
    for arm in CHURN_ARMS:
        for mode in ("sleeping_off", "sleeping_on"):
            k = "row_identity_churn/%s/%s_tree/tree" % (arm, mode)
            got = json.loads(sorted(diffs[k])[0])
            exp = (0 if arm == "stable" else 4, 0, 1, 1)
            rec_check.append((k, "translations +%d evictions 0 static_rebuilds 1 members 1" % exp[0], (got["translations"], got["evictions"], got["static_rebuilds"], got["members"]),
                              (got["translations"], got["evictions"], got["static_rebuilds"], got["members"]) == exp))
    p("== G4 receipts against the recipe's dry run:")
    for key, exp, got, ok in rec_check:
        p("   %-52s expected %-45s got %-40s %s" % (key, exp, got, "ok" if ok else "DIFFERS"))

    # --- cells ---------------------------------------------------------------------------
    cells = OrderedDict()
    for k in sorted(vals):
        lst = vals[k]
        c = cell([v / 1e6 for v, _, _ in lst])  # ms
        c["baselines"] = [b for _, b, _ in sorted(lst)]
        c["criterion_se_pct_median"] = statistics.median(se / v * 100 for v, _, se in lst)
        cells[k] = c

    def C(k):
        return cells[k]

    p("\n== G4 pair-finding cells (median over K=3 of criterion median.point_estimate, ms; [min-max]; range % / SE %)")
    for grp in ("bp_g4_uniform", "bp_g4_disparity", "bp_g4_scene"):
        params = G4_SIZES if grp != "bp_g4_scene" else [1240, 10000, 100000, "j100"]
        p("  %s" % grp)
        for prm in params:
            row = []
            for arm in ARMS:
                c = C("%s/%s/%s" % (grp, arm, prm))
                row.append("%s %.5f [%.5f-%.5f] %4.2f/%4.2f" % (arm, c["median"], c["min"], c["max"], c["range_pct"], c["se_pct"]))
            p("    n=%-7s %s" % (prm, " | ".join(row)))
    p("\n== G4 maintenance cells (ms)")
    for m in MS:
        for arm in MAINT:
            c = C("bp_g4_maintenance/%s/%d" % (arm, m))
            p("    m=%-6d %-22s %.6f [%.6f-%.6f] range %.2f%% SE %.2f%%  values %s" % (m, arm, c["median"], c["min"], c["max"], c["range_pct"], c["se_pct"], ", ".join("%.6f" % v for v in c["values"])))
    p("\n== G4 churn cells (K=6, ms per churn step)")
    for arm in CHURN_ARMS:
        for mode in ("sleeping_off", "sleeping_on"):
            for bp in ("allpairs", "tree"):
                c = C("row_identity_churn/%s/%s_%s" % (arm, mode, bp))
                p("    %-22s %-12s %-8s %.5f [%.5f-%.5f] range %.2f%% IQR %.2f%% SE %.2f%%  values %s" % (arm, mode, bp, c["median"], c["min"], c["max"], c["range_pct"], c["iqr_pct"], c["se_pct"], ", ".join("%.5f" % v for v in c["values"])))

    # --- comparisons: tree vs all_pairs and tree vs grid_w1 per n and family ------------
    R2 = ("range", "se")
    comps = OrderedDict()
    p("\n== G4 tree against all_pairs and grid_w1 per n (B against A; claimed under range / SE)")
    for grp in ("bp_g4_uniform", "bp_g4_disparity", "bp_g4_scene"):
        params = G4_SIZES if grp != "bp_g4_scene" else [1240, 10000, 100000, "j100"]
        for prm in params:
            t = C("%s/tree/%s" % (grp, prm))
            for arm in ("all_pairs", "grid_w1", "grid_w8"):
                a = C("%s/%s/%s" % (grp, arm, prm))
                c = compare(a, t, R2)
                comps["%s/%s: tree vs %s" % (grp, prm, arm)] = c
                # and the reverse reading for the crossover rule: all_pairs against tree
                if arm == "all_pairs":
                    c2 = compare(t, a, R2)
                    comps["%s/%s: all_pairs vs tree" % (grp, prm)] = c2
            ca, cg = comps["%s/%s: tree vs all_pairs" % (grp, prm)], comps["%s/%s: tree vs grid_w1" % (grp, prm)]
            cr = comps["%s/%s: all_pairs vs tree" % (grp, prm)]
            p("  %-18s n=%-7s tree/all_pairs %.4f (%+7.1f %%) bars %5.2f/%5.2f %s   all_pairs/tree %.4f (%+7.1f %%) %s   tree/grid_w1 %.4f (%+7.1f %%) bars %5.2f/%5.2f %s   tree/grid_w8 %.4f" % (
                grp, prm, ca["ratio"], ca["effect_pct"], ca["bars"]["range"], ca["bars"]["se"], fmt_claim(ca, R2), cr["ratio"], cr["effect_pct"], fmt_claim(cr, R2),
                cg["ratio"], cg["effect_pct"], cg["bars"]["range"], cg["bars"]["se"], fmt_claim(cg, R2), comps["%s/%s: tree vs grid_w8" % (grp, prm)]["ratio"]))

    # --- 1.2 stop rules ------------------------------------------------------------------
    p("\n== 1.2 stop rules (medians, ms)")
    stop = OrderedDict()

    def rule(name, cellname, value, limit, extra=""):
        fired = value > limit
        stop[name if cellname is None else "%s [%s]" % (name, cellname)] = OrderedDict(cell=cellname, value_ms=value, limit_ms=limit, fired=fired, ratio=value / limit if limit else None)
        p("   %-16s %-38s %.6f vs %.6f  (%.2fx)  %s %s" % (name, cellname, value, limit, value / limit, "FIRED" if fired else "held", extra))

    rule("J snapshot", "bp_g4_scene/tree/j100", C("bp_g4_scene/tree/j100")["median"], 0.30)
    tvg = []
    for grp in ("bp_g4_uniform", "bp_g4_disparity", "bp_g4_scene"):
        params = [n for n in (G4_SIZES if grp != "bp_g4_scene" else [1240, 10000, 100000]) if n > 64]
        for prm in params:
            c = comps["%s/%s: tree vs grid_w1" % (grp, prm)]
            slower_claimed = c["effect_pct"] > 0 and (c["claimed"]["range"] or c["claimed"]["se"])
            tvg.append((grp, prm, c["ratio"], slower_claimed))
    fired = any(x[3] for x in tvg)
    stop["Tree vs Grid"] = OrderedDict(fired=fired, ratios=[(g, n, round(r, 4), s) for g, n, r, s in tvg])
    p("   %-16s tree/grid_w1 at n > 64 (uniform, disparity, scene): %s -> %s" % ("Tree vs Grid", ", ".join("%s/%s %.3f" % (g.replace("bp_g4_", ""), n, r) for g, n, r, _ in tvg), "FIRED" if fired else "held (tree never slower)"))
    limits = OrderedDict(
        stable=[0.058, 0.46, 4.6], translation=[0.062, 0.5, 5.0], eviction=[0.048, 0.38, 3.8], compaction=[0.050, 0.40, 6.0],
        admission_of_64=[0.120, 0.80, 9.8], from_empty=[0.46, 4.8, 60.0], high_jumper=[0.062, 0.5, 5.0])
    for i, m in enumerate(MS):
        st = C("bp_g4_maintenance/stable/%d" % m)["median"]
        ev = C("bp_g4_maintenance/eviction_filter/%d" % m)["median"]
        rule("stable", "stable/%d" % m, st, limits["stable"][i])
        rule("translation", "shift_translation/%d - stable" % m, C("bp_g4_maintenance/shift_translation/%d" % m)["median"] - st, limits["translation"][i])
        rule("eviction", "eviction_filter/%d - stable" % m, ev - st, limits["eviction"][i])
        rule("compaction", "compaction/%d - eviction_filter" % m, C("bp_g4_maintenance/compaction/%d" % m)["median"] - ev, limits["compaction"][i],
             "(compaction - stable = %.6f; from_empty - stable = %.6f)" % (C("bp_g4_maintenance/compaction/%d" % m)["median"] - st, C("bp_g4_maintenance/admission_from_empty/%d" % m)["median"] - st))
        rule("admission of 64", "admission_of_64/%d - stable" % m, C("bp_g4_maintenance/admission_of_64/%d" % m)["median"] - st, limits["admission_of_64"][i])
        rule("from empty", "admission_from_empty/%d - stable" % m, C("bp_g4_maintenance/admission_from_empty/%d" % m)["median"] - st, limits["from_empty"][i])
        hj = C("bp_g4_maintenance/high_jumper/%d" % m)
        rule("high jumper W1", "high_jumper/%d - stable" % m, hj["median"] - st, limits["high_jumper"][i],
             "(high_jumper min-max %.6f-%.6f, stable %.6f [%.6f-%.6f]; min-max of the difference %.6f..%.6f)" % (
                 hj["min"], hj["max"], st, C("bp_g4_maintenance/stable/%d" % m)["min"], C("bp_g4_maintenance/stable/%d" % m)["max"],
                 hj["min"] - C("bp_g4_maintenance/stable/%d" % m)["max"], hj["max"] - C("bp_g4_maintenance/stable/%d" % m)["min"]))

    # --- churn claim rules ---------------------------------------------------------------
    p("\n== 1.1 churn claim rules (K=6): tree faster than allpairs per arm, claimed (range / IQR / SE); tree(arm) - tree(stable) <= 0.05 ms")
    churn = OrderedDict()
    for mode in ("sleeping_off", "sleeping_on"):
        ts = C("row_identity_churn/stable/%s_tree" % mode)
        for arm in CHURN_ARMS:
            a = C("row_identity_churn/%s/%s_allpairs" % (arm, mode))
            t = C("row_identity_churn/%s/%s_tree" % (arm, mode))
            c = compare(a, t)
            d = t["median"] - ts["median"]
            dc = compare(ts, t)
            as_ = C("row_identity_churn/stable/%s_allpairs" % mode)
            da = a["median"] - as_["median"]
            churn["%s/%s" % (arm, mode)] = OrderedDict(allpairs=a["median"], tree=t["median"], cmp=c, tree_minus_stable_ms=d, tree_vs_stable=dc,
                                                     allpairs_minus_stable_ms=da, did_ms=d - da,
                                                     faster_claimed_range_and_se=c["effect_pct"] < 0 and c["claimed"]["range"] and c["claimed"]["se"],
                                                     faster_claimed_se=c["effect_pct"] < 0 and c["claimed"]["se"],
                                                     within_0_05=d <= 0.05)
            p("   %-22s %-12s allpairs %.5f tree %.5f  tree/allpairs %.4f (%+6.2f %%) bars %s %s | tree-stable %+.5f ms (%s; vs stable %+.2f %% %s) | allpairs-stable %+.5f | DiD %+.5f" % (
                arm, mode, a["median"], t["median"], c["ratio"], c["effect_pct"], fmt_bars(c), fmt_claim(c), d, "<= 0.05" if d <= 0.05 else "ABOVE 0.05", dc["effect_pct"], fmt_claim(dc), da, d - da))

    # --- 1.4 constants -------------------------------------------------------------------
    p("\n== 1.4 C4 constants")
    small = [17, 64, 128, 256, 1000]
    cross = OrderedDict()
    for grp in ("bp_g4_uniform", "bp_g4_disparity"):
        per_n = OrderedDict()
        for n in small:
            ap_vs_tree = comps["%s/%s: all_pairs vs tree" % (grp, n)]      # all_pairs against tree: +% means all_pairs slower
            tree_vs_ap = comps["%s/%s: tree vs all_pairs" % (grp, n)]
            ap_slower_claimed = ap_vs_tree["effect_pct"] > 0 and (ap_vs_tree["claimed"]["range"] or ap_vs_tree["claimed"]["se"])
            ap_slower_claimed_both = ap_vs_tree["effect_pct"] > 0 and ap_vs_tree["claimed"]["range"] and ap_vs_tree["claimed"]["se"]
            tree_faster_claimed = tree_vs_ap["effect_pct"] < 0 and (tree_vs_ap["claimed"]["range"] or tree_vs_ap["claimed"]["se"])
            tree_faster_claimed_both = tree_vs_ap["effect_pct"] < 0 and tree_vs_ap["claimed"]["range"] and tree_vs_ap["claimed"]["se"]
            per_n[n] = OrderedDict(all_pairs=C("%s/all_pairs/%s" % (grp, n))["median"], tree=C("%s/tree/%s" % (grp, n))["median"],
                                   ap_over_tree=ap_vs_tree["ratio"], ap_slower_claimed_either=ap_slower_claimed, ap_slower_claimed_both=ap_slower_claimed_both,
                                   tree_faster_claimed_either=tree_faster_claimed, tree_faster_claimed_both=tree_faster_claimed_both,
                                   bars=ap_vs_tree["bars"], tree_bars=tree_vs_ap["bars"], effect_pct=ap_vs_tree["effect_pct"], tree_effect_pct=tree_vs_ap["effect_pct"])
        not_slower = [n for n in small if not per_n[n]["ap_slower_claimed_either"]]
        brute_max = max(not_slower) if not_slower else None
        hi_cands = [n for n in small if per_n[n]["tree_faster_claimed_either"]]
        auto_hi = min(hi_cands) if hi_cands else None
        # monotonic sanity: every n above brute_max must have all_pairs claimed slower, every n below auto_hi not tree-faster
        mono = all(per_n[n]["ap_slower_claimed_either"] for n in small if brute_max is not None and n > brute_max) and \
            all(not per_n[n]["tree_faster_claimed_either"] for n in small if auto_hi is not None and n < auto_hi)
        # log-log interpolation of the ratio all_pairs/tree crossing 1 (L2's method for GRID_HI)
        interp = None
        for a_, b_ in zip(small, small[1:]):
            ra, rb = per_n[a_]["ap_over_tree"], per_n[b_]["ap_over_tree"]
            if (ra - 1) * (rb - 1) < 0:
                la, lb, lra, lrb = math.log(a_), math.log(b_), math.log(ra), math.log(rb)
                interp = math.exp(la + (0 - lra) * (lb - la) / (lrb - lra))
                break
        cross[grp] = OrderedDict(per_n=per_n, TREE_BRUTE_MAX_ROWS=brute_max, AUTO_TREE_LO=brute_max, AUTO_TREE_HI=auto_hi, monotone=mono, loglog_crossover=interp)
        p("  %s: " % grp + "; ".join("n=%d ap/tree %.3f (%+.1f %%, bars %.1f/%.1f; ap slower claimed %s/%s; tree faster claimed %s/%s)" % (
            n, per_n[n]["ap_over_tree"], per_n[n]["effect_pct"], per_n[n]["bars"]["range"], per_n[n]["bars"]["se"],
            "Y" if per_n[n]["ap_slower_claimed_either"] else "n", "Y" if per_n[n]["ap_slower_claimed_both"] else "n",
            "Y" if per_n[n]["tree_faster_claimed_either"] else "n", "Y" if per_n[n]["tree_faster_claimed_both"] else "n") for n in small))
        p("     -> largest n with all_pairs not claimed slower = %s; smallest n with tree claimed faster = %s; monotone %s; log-log crossover %s" % (
            brute_max, auto_hi, mono, "%.0f" % interp if interp else None))
    tbmr = min(cross[g]["TREE_BRUTE_MAX_ROWS"] for g in cross)
    lo = min(cross[g]["AUTO_TREE_LO"] for g in cross)
    hi = max(cross[g]["AUTO_TREE_HI"] for g in cross)
    lo = max(lo, tbmr)
    p("  TREE_BRUTE_MAX_ROWS = min(uniform, disparity) = %d (today 64); AUTO_TREE_LO = %d, AUTO_TREE_HI = %d (the wider band; LO >= TREE_BRUTE_MAX_ROWS); log-log crossovers uniform %s / disparity %s" % (
        tbmr, lo, hi, "%.0f" % cross["bp_g4_uniform"]["loglog_crossover"] if cross["bp_g4_uniform"]["loglog_crossover"] else None,
        "%.0f" % cross["bp_g4_disparity"]["loglog_crossover"] if cross["bp_g4_disparity"]["loglog_crossover"] else None))

    # ADMIT_BUILD_RATIO
    p("\n  ADMIT_BUILD_RATIO = (c_build + k*c_list) / c_q; c_q = t_q(J)/1240 = %.6f ms / 1240 = %.2f ns/row (check: uniform/tree/10000 / 10000 = %.2f ns/row)" % (
        tq_ms, tq_ms * 1e6 / 1240, C("bp_g4_uniform/tree/10000")["median"] * 1e6 / 10000))
    abr = OrderedDict()
    cq_J = tq_ms * 1e6 / 1240
    cq_10k = C("bp_g4_uniform/tree/10000")["median"] * 1e6 / 10000
    for m in MS:
        st = C("bp_g4_maintenance/stable/%d" % m)["median"]
        L = RECIPE_STABLE_PAIRS[m]
        c_build = (C("bp_g4_maintenance/admission_from_empty/%d" % m)["median"] - st) * 1e6 / m
        c_list = (C("bp_g4_maintenance/eviction_filter/%d" % m)["median"] - st) * 1e6 / L
        k = L / m
        num = c_build + k * c_list
        abr[m] = OrderedDict(c_build_ns=c_build, c_list_ns=c_list, k=k, numerator_ns=num, ratio_cq_J=num / cq_J, ratio_cq_10k=num / cq_10k,
                             ratio_cq_design_mid=num / {1240: 125, 10000: 175, 100000: 250}[m])
        p("   m=%-6d c_build %.2f ns/row  c_list %.3f ns/entry  k=%.2f  num %.2f ns/row  ratio: /c_q(J) %.4f  /c_q(10k bound) %.4f  /design c_q mid (%d ns) %.4f" % (
            m, c_build, c_list, k, num, num / cq_J, num / cq_10k, {1240: 125, 10000: 175, 100000: 250}[m], abr[m]["ratio_cq_design_mid"]))
    p("\n  the compaction arm's timed step re-queries h = ceil(m/2) evicted rows as Q rows (benches/broadphase.rs, MaintArm::Compaction); residual = (compaction - eviction_filter) - (h-1)*c_q:")
    resid = OrderedDict()
    for m in MS:
        h = -(-m // 2)
        ev = C("bp_g4_maintenance/eviction_filter/%d" % m)["median"]
        cp = C("bp_g4_maintenance/compaction/%d" % m)["median"]
        st = C("bp_g4_maintenance/stable/%d" % m)["median"]
        per_row_bound = {1240: cq_J, 10000: cq_10k, 100000: C("bp_g4_uniform/tree/100000")["median"] * 1e6 / 100000}[m]
        q_cost = (h - 1) * per_row_bound / 1e6
        design = {1240: (0.015, 0.025), 10000: (0.12, 0.20), 100000: (2.0, 3.0)}[m]
        resid[m] = OrderedDict(h=h, compaction_minus_ev_ms=cp - ev, c_q_used_ns=per_row_bound, h_queries_ms=q_cost, residual_ms=cp - ev - q_cost, design_band_ms=design,
                               per_live_leaf_ns=(cp - ev) * 1e6 / h, half_from_empty_ms=0.5 * (C("bp_g4_maintenance/admission_from_empty/%d" % m)["median"] - st))
        p("   m=%-6d h=%-6d compaction-ev %.4f ms = %.0f ns per live leaf; (h-1)*c_q at %.0f ns/row = %.4f ms; residual %.4f ms vs design compaction %s ms; 0.5*(from_empty-stable) = %.4f" % (
            m, h, cp - ev, (cp - ev) * 1e6 / h, per_row_bound, q_cost, cp - ev - q_cost, design, resid[m]["half_from_empty_ms"]))
    p("  c_build bounded from admission_of_64 (timed step = radix rebuild over m+64 + 64 queries + merge): (a64 - stable - 64*c_q(J)) / (m+64) is an UPPER bound on the radix c_build:")
    cb = OrderedDict()
    for m in MS:
        st = C("bp_g4_maintenance/stable/%d" % m)["median"]
        a64 = C("bp_g4_maintenance/admission_of_64/%d" % m)["median"]
        ev = C("bp_g4_maintenance/eviction_filter/%d" % m)["median"]
        ub = (a64 - st - 64 * cq_J / 1e6) * 1e6 / (m + 64)
        ub_less_merge = (a64 - st - 64 * cq_J / 1e6 - (ev - st)) * 1e6 / (m + 64)
        L = RECIPE_STABLE_PAIRS[m]
        k = L / m
        c_list = (ev - st) * 1e6 / L
        cb[m] = OrderedDict(c_build_upper_ns=ub, c_build_upper_less_one_list_pass_ns=ub_less_merge, ratio_upper=(ub + k * c_list) / cq_J, ratio_upper_less_merge=(max(ub_less_merge, 0) + k * c_list) / cq_J,
                            ratio_design_cq=(ub + k * c_list) / {1240: 125, 10000: 175, 100000: 250}[m])
        p("   m=%-6d c_build <= %.1f ns/row (<= %.1f if the merge is one list pass); (c_build + k*c_list)/c_q(J) <= %.3f (%.3f); with the design's mid c_q: %.3f" % (
            m, ub, ub_less_merge, cb[m]["ratio_upper"], cb[m]["ratio_upper_less_merge"], cb[m]["ratio_design_cq"]))
    # (num, den) with den <= 8 nearest to the m=10k value
    best = None
    for den in range(1, 9):
        for numr in range(0, 3 * den + 1):
            v = numr / den
            if best is None or abs(v - abr[10000]["ratio_cq_J"]) < abs(best[0] - abr[10000]["ratio_cq_J"]):
                best = (v, numr, den)
    best10 = None
    for den in range(1, 9):
        for numr in range(0, 3 * den + 1):
            v = numr / den
            if best10 is None or abs(v - abr[10000]["ratio_cq_10k"]) < abs(best10[0] - abr[10000]["ratio_cq_10k"]):
                best10 = (v, numr, den)
    p("   m=10k: ratio %.4f -> nearest (num, den) with den <= 8: (%d, %d) = %.4f; with the 10k-bound c_q: %.4f -> (%d, %d)" % (
        abr[10000]["ratio_cq_J"], best[1], best[2], best[0], abr[10000]["ratio_cq_10k"], best10[1], best10[2]))

    return OrderedDict(selection=OrderedDict(used=len(used), why={"%s/k%d/%d/%s" % k: v for k, v in why.items() if v != "original"}, excluded=[list(k) for k in excluded]),
                       receipts_check=rec_check, non_identical_receipts=non_identical, cells=cells, comparisons=comps, stop_rules=stop, churn=churn,
                       crossover=cross, constants=OrderedDict(TREE_BRUTE_MAX_ROWS=tbmr, AUTO_TREE_LO=lo, AUTO_TREE_HI=hi, ADMIT_BUILD_RATIO=abr, nearest_num_den_cqJ=best, nearest_num_den_cq10k=best10, compaction_residual=resid, c_build_bound=cb))


def main():
    sums = {}
    for line in open(os.path.join(W, "bin", "SHA256SUMS"), encoding="utf-8"):
        h, name = line.split(None, 1)
        sums[name.strip().lstrip("*")] = h
    p("== bin/SHA256SUMS: %s" % {os.path.basename(k): v[:12] for k, v in sums.items()})
    runner = os.path.join(W, "bin", "runner_c3.exe")
    if os.path.exists(runner):
        p("== runner_c3.exe sha256 now: %s (%s)" % (sha256_file(runner)[:12], "matches" if sha256_file(runner) == sums["runner_c3.exe"] else "DIFFERS"))
    r5 = g5()
    tq = r5["d6"]["t_q_ms"]
    r4 = g4(tq)
    out = OrderedDict(g5=r5, g4=r4, sha256sums=sums)
    with open(os.path.join(OUT, "reduction.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, default=str)
    with open(os.path.join(OUT, "tables.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(LINES) + "\n")
    p("\nwrote %s and %s" % (os.path.join(OUT, "reduction.json"), os.path.join(OUT, "tables.txt")))


if __name__ == "__main__":
    main()
