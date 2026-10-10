"""Reduce the armed per-step CSVs of receipts.sh to the window-9 work receipts over steps
[100, 500): manifolds, points (the solved slots: the sum of the phys_hist_slots_* counters), colours
(the armed `colors` column), colour passes (colours x 12 = substeps 4 x (1 + relax 2)) and velocity
rows (points x 12). Untimed: every column read is a count."""
import csv
import os
import statistics as st
import sys

OUT = sys.argv[1]
SWEEPS = 12
ROWS = ["JT_parent", "JT_d0", "JT_v2", "JToff_parent", "JToff_d0", "JToff_v2"]


def reduce(name):
    with open(os.path.join(OUT, name + ".csv")) as f:
        rows = list(csv.DictReader(f))[100:500]
    slot_cols = [k for k in rows[0] if k.startswith("phys_hist_slots") and not k.endswith("_n")]
    assert slot_cols, f"{name}: no phys_hist_slots_* counter columns (was the row armed?)"
    m = st.mean(float(r["manifolds"]) for r in rows)
    p = st.mean(sum(float(r[k] or 0) for k in slot_cols) for r in rows)
    c = st.mean(float(r["colors"]) for r in rows)
    return {"steps": len(rows), "manifolds": m, "points": p, "colours": c,
            "passes": c * SWEEPS, "rows": p * SWEEPS}


res = {n: reduce(n) for n in ROWS if os.path.exists(os.path.join(OUT, n + ".csv"))}
print(f"{'row':14s} {'steps':>5s} {'manifolds':>10s} {'points':>9s} {'colours':>8s} {'passes':>7s} {'vel rows':>9s} {'rows x':>7s} {'passes x':>8s}")
for n, r in res.items():
    base = res.get(n.split("_")[0] + "_parent", r)
    print(f"{n:14s} {r['steps']:5d} {r['manifolds']:10.2f} {r['points']:9.2f} {r['colours']:8.3f} "
          f"{r['passes']:7.2f} {r['rows']:9.1f} {r['rows'] / base['rows']:7.3f} {r['passes'] / base['passes']:8.3f}")
for scene in ("JT", "JToff"):
    if f"{scene}_d0" in res and f"{scene}_parent" in res:
        same = res[f"{scene}_d0"] == res[f"{scene}_parent"]
        print(f"{scene}: the overlap-only rule's receipts equal the parent's: {same}")
