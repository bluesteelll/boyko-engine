"""Reference implementation of window 9a's void rules V1-V9 for ONE timed Rapier process.

The window driver applies these rules to every timed process; this file is their executable form,
so the rules are known to be checkable from what a timed command line (no `--receipt`) actually
produces: one log with a `SUMMARY {json}` line, the per-step CSV, the final pose file, the exe.
`analyze.py` runs it on every timed-shape twin (it must be clean) and on mutated copies (each
mutation must turn exactly the rules it targets red): the red controls of the validator.

    python void_check.py --pins pins.json --arm simd8 --cfg rapier-default --workers 8 \
        --log run.log --csv run.csv --pose final.pose --exe bin/rapier-parity-simd8.exe

Exit 0: valid. Exit 1: void (the violated rules and reasons are printed). No timing value is
read except the boolean `wall_ns > 0` of V8; nothing is printed from a timing column.

The rules (RP rows of `window9a_rows.md`):
  V1  exit != 0 (2 usage, 3 receipt gate, 4 pose mismatch), a crash, or a hang (> 300 s, killed by the driver)
  V2  not exactly one parsable SUMMARY line
  V3  pool-size receipt != W: SUMMARY pool_threads_min == pool_threads_max == W, install_receipt == [W, W],
      loop_on_pool_worker, and CSV pool_threads == W on all 500 rows
  V4  build receipt differs: rapier3d 0.36.0, the arm's features, arm and lane count, avx2/fma/bmi2, no
      debug assertions, the exe sha256 and the source hashes equal their pins
  V5  the `config` object differs from the cfg's timed pin in any field (counters_enabled included)
  V6  scene identity not all-true, spawn hash != pin, or perturb not null
  V7  final pose != the arm's fixture
  V8  the CSV is not exactly `step,wall_ns,pool_threads` + 500 rows (steps 0..499) with wall_ns > 0
  V9  count receipts: the row ran without --receipt, and the prep twin of its (cfg, arm, W) cell passed
"""
import argparse
import hashlib
import json
import sys

STEPS = 500
BODIES = 1240
TIMED_CSV_HEADER = "step,wall_ns,pool_threads"


def sha256_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def check(pins, arm, cfg, workers, log_text, csv_text, pose_raw, fixture_raw, exe_sha256, rc, timed_out=False):
    """Returns {rule: [reasons]}; an empty dict means the process is valid."""
    bad = {}

    def flag(rule, why):
        bad.setdefault(rule, []).append(why)

    a = pins["arms"][arm]
    c = a["cfgs"][cfg]

    # V1
    if timed_out:
        flag("V1", "hang: terminated by the driver")
    if rc != 0:
        flag("V1", f"exit code {rc}")

    # V2
    lines = [l for l in log_text.splitlines() if l.startswith("SUMMARY ")]
    s = None
    if len(lines) != 1:
        flag("V2", f"{len(lines)} SUMMARY lines")
    else:
        try:
            s = json.loads(lines[0][len("SUMMARY "):])
        except ValueError as e:
            flag("V2", f"SUMMARY is not JSON: {e}")

    # CSV, parsed once for V3 / V8
    csv_lines = csv_text.splitlines()
    head = csv_lines[0] if csv_lines else ""
    rows = [l.split(",") for l in csv_lines[1:]]

    # V8
    if head != TIMED_CSV_HEADER:
        flag("V8", f"CSV header {head!r}")
    if len(rows) != STEPS:
        flag("V8", f"{len(rows)} CSV rows, want {STEPS}")
    try:
        if [int(r[0]) for r in rows] != list(range(len(rows))):
            flag("V8", "CSV steps are not 0..N-1")
        if any(int(r[1]) <= 0 for r in rows):
            flag("V8", "a CSV row has wall_ns <= 0")
    except (ValueError, IndexError):
        flag("V8", "CSV rows are not parsable")

    # V3 (the CSV clause runs even when the SUMMARY did not parse)
    try:
        pool_col = [int(r[2]) for r in rows]
        if len(pool_col) != STEPS or any(n != workers for n in pool_col):
            flag("V3", f"CSV pool_threads is not {workers} on all {STEPS} rows")
    except (ValueError, IndexError):
        flag("V3", "CSV pool_threads column missing or unparsable")

    # V7 (independent of the SUMMARY)
    if pose_raw != fixture_raw:
        flag("V7", "final pose bytes != fixture")

    if s is None:
        return bad

    th = s.get("threads", {})
    if th.get("pool_threads_min") != workers or th.get("pool_threads_max") != workers:
        flag("V3", f"pool_threads_min/max {th.get('pool_threads_min')}/{th.get('pool_threads_max')}, want {workers}")
    if th.get("install_receipt") != [workers, workers]:
        flag("V3", f"install_receipt {th.get('install_receipt')}, want {[workers, workers]}")
    if th.get("loop_on_pool_worker") is not True:
        flag("V3", "loop_on_pool_worker is not true")
    if th.get("pool_threads_requested") != workers or s.get("workers") != workers:
        flag("V3", "requested worker count differs from W")

    # V4
    if s.get("engine") != "rapier3d" or s.get("rapier_version") != pins["rapier_version"]:
        flag("V4", f"engine/version {s.get('engine')} {s.get('rapier_version')}")
    if s.get("features") != a["features"]:
        flag("V4", f"features {s.get('features')}, want {a['features']}")
    if s.get("arm") != arm or s.get("simd_lanes") != a["lanes"]:
        flag("V4", f"arm/lanes {s.get('arm')}/{s.get('simd_lanes')}, want {arm}/{a['lanes']}")
    if s.get("target_features") != {"avx2": True, "fma": True, "bmi2": True}:
        flag("V4", f"target_features {s.get('target_features')}")
    if s.get("debug_assertions") is not False or s.get("target_env") != "msvc":
        flag("V4", "debug assertions on, or not msvc")
    if exe_sha256 != pins["exe_sha256"][arm]:
        flag("V4", f"exe sha256 {exe_sha256} != pin {pins['exe_sha256'][arm]}")
    if s.get("source_fnv1a64") != pins["source_fnv1a64"]:
        flag("V4", "source hashes differ from the pinned sources")

    # V5
    if s.get("cfg") != cfg or s.get("install") != "loop":
        flag("V5", f"cfg/install {s.get('cfg')}/{s.get('install')}")
    if s.get("config") != c["config_timed"]:
        diff = sorted(k for k in set(s.get("config", {})) | set(c["config_timed"])
                      if s.get("config", {}).get(k) != c["config_timed"].get(k))
        flag("V5", f"config differs from the pin in {diff}")

    # V6
    si = s.get("scene_identity", {})
    ok = (si.get("dynamic_bodies") == BODIES and si.get("fixed_bodies") == 1 and si.get("colliders") == BODIES + 1
          and si.get("first_equals_ours") is True and si.get("last_equals_ours") is True
          and si.get("aabb_equals_expected") is True and si.get("uniform_boxes") is True
          and si.get("inv_mass_ulps_vs_ours") == 0 and si.get("inv_inertia_ulps_vs_ours") == [0, 0, 0])
    if not ok:
        flag("V6", "scene identity is not all-true")
    if si.get("spawn_hash") != pins["spawn_hash"]:
        flag("V6", f"spawn hash {si.get('spawn_hash')}")
    if s.get("perturb") is not None:
        flag("V6", "perturb is not null")
    if s.get("void") is not False:
        flag("V6", f"the run itself is void: {s.get('voids')}")

    # V7 (the harness's own verdict and hash, beside the byte compare above)
    if s.get("expect_pose") != "match" or s.get("pose_hash") != c["pose_hash"]:
        flag("V7", f"expect_pose {s.get('expect_pose')!r}, pose_hash {s.get('pose_hash')}")

    # V9
    if "--receipt" in s.get("args", []) or s.get("receipt_gates", {}).get("awake_min") is not None:
        flag("V9", "a timed row ran with --receipt")
    if c["twin_ok"].get(str(workers)) is not True:
        flag("V9", f"the prep twin of ({arm}, {cfg}, W{workers}) did not pass")
    return bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pins", required=True)
    ap.add_argument("--arm", required=True)
    ap.add_argument("--cfg", required=True)
    ap.add_argument("--workers", type=int, required=True)
    ap.add_argument("--log", required=True)
    ap.add_argument("--csv", required=True)
    ap.add_argument("--pose", required=True)
    ap.add_argument("--exe", required=True)
    ap.add_argument("--rc", type=int, help="exit code; default: the `rc=` line the gate scripts append to a log")
    ap.add_argument("--timed-out", action="store_true")
    a = ap.parse_args()
    pins = json.load(open(a.pins, encoding="utf-8"))
    log = open(a.log, encoding="utf-8", errors="replace").read()
    rc = a.rc
    if rc is None:
        rc = next((int(l[3:]) for l in reversed(log.splitlines()) if l.startswith("rc=")), -1)
    fixture = open(pins["arms"][a.arm]["cfgs"][a.cfg]["fixture_path"], "rb").read()
    bad = check(pins, a.arm, a.cfg, a.workers, log, open(a.csv, encoding="utf-8").read(), open(a.pose, "rb").read(),
                fixture, sha256_bytes(open(a.exe, "rb").read()), rc, a.timed_out)
    for rule in sorted(bad):
        for why in bad[rule]:
            print(f"{rule}: {why}")
    print("VOID" if bad else "VALID")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
