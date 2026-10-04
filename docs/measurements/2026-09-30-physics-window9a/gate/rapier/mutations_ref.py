"""The 33 red-control mutations of the harness's reference validator (void_check.py), extracted VERBATIM from
D:/tmp/rapier-parity/gate/analyze.py (section "gv"; lines 456-540 of the file as of 2026-09-30, harness rev 2). A byte-for-byte
excerpt so the window can re-run them without importing analyze.py (a script that runs the whole harness gate and rewrites the
harness dir). Each entry: (name, rules it must turn red, rules it may also turn red, fn(ctx) -> mutates the ctx dict in place).
ctx keys: pins arm cfg workers log_text csv_text pose_raw fixture_raw exe_sha256 rc timed_out."""
import json


def edit_summary(log_text, fn):
    lines = log_text.splitlines()
    i = next(i for i, l in enumerate(lines) if l.startswith("SUMMARY "))
    d = json.loads(lines[i][len("SUMMARY "):])
    fn(d)
    lines[i] = "SUMMARY " + json.dumps(d, separators=(",", ":"))
    return "\n".join(lines) + "\n"


def summ(fn):
    return lambda c: c.update(log_text=edit_summary(c["log_text"], fn))


def csv_edit(fn):
    def go(c):
        lines = c["csv_text"].splitlines()
        c["csv_text"] = "\n".join(fn(lines)) + "\n"
    return go


def other_arm(a):
    return "simd4" if a == "simd8" else "simd8"


def flip_pose(c):
    b = bytearray(c["pose_raw"])
    b[52 * 617] ^= 0x01
    c["pose_raw"] = bytes(b)


def set_twin(c):
    c["pins"]["arms"][c["arm"]]["cfgs"][c["cfg"]]["twin_ok"][str(c["workers"])] = False


def set_other_config(c):
    other = "matched" if c["cfg"] == "rapier-default" else "rapier-default"
    cfg_other = c["pins"]["arms"][c["arm"]]["cfgs"][other]["config_timed"]
    c["log_text"] = edit_summary(c["log_text"], lambda d: d.update(config=cfg_other))


def row_edit(i, col, val):
    def f(lines):
        cells = lines[1 + i].split(",")
        cells[col] = str(val)
        lines[1 + i] = ",".join(cells)
        return lines
    return f


MUTATIONS = [
    ("V1 exit code 4", {"V1"}, set(), lambda c: c.update(rc=4)),
    ("V1 exit code 3", {"V1"}, set(), lambda c: c.update(rc=3)),
    ("V1 hang", {"V1"}, set(), lambda c: c.update(timed_out=True)),
    ("V2 no SUMMARY line", {"V2"}, set(), lambda c: c.update(log_text="\n".join(l for l in c["log_text"].splitlines() if not l.startswith("SUMMARY ")))),
    ("V2 two SUMMARY lines", {"V2"}, set(), lambda c: c.update(log_text=c["log_text"] + next(l for l in c["log_text"].splitlines() if l.startswith("SUMMARY ")) + "\n")),
    ("V2 unparsable SUMMARY", {"V2"}, set(), lambda c: c.update(log_text=c["log_text"].replace("SUMMARY {", "SUMMARY {{"))),
    ("V3 pool_threads_max = W+1", {"V3"}, set(), summ(lambda d: d["threads"].update(pool_threads_max=d["workers"] + 1))),
    ("V3 pool_threads_min = W+1", {"V3"}, set(), summ(lambda d: d["threads"].update(pool_threads_min=d["workers"] + 1))),
    ("V3 install_receipt = [W, W+1]", {"V3"}, set(), summ(lambda d: d["threads"].update(install_receipt=[d["workers"], d["workers"] + 1]))),
    ("V3 loop_on_pool_worker false", {"V3"}, set(), summ(lambda d: d["threads"].update(loop_on_pool_worker=False))),
    ("V3 CSV pool_threads row 250 = W+1", {"V3"}, set(), lambda c: csv_edit(row_edit(250, 2, c["workers"] + 1))(c)),
    ("V3 CSV pool_threads column absent (the rev-1 timed CSV)", {"V3", "V8"}, set(),
     csv_edit(lambda ls: [",".join(l.split(",")[:2]) for l in ls])),
    ("V4 features.simd8 flipped", {"V4"}, set(), summ(lambda d: d["features"].update(simd8=not d["features"]["simd8"]))),
    ("V4 features.simd4 flipped", {"V4"}, set(), summ(lambda d: d["features"].update(simd4=not d["features"]["simd4"]))),
    ("V4 features.enhanced_determinism true", {"V4"}, set(), summ(lambda d: d["features"].update(enhanced_determinism=True))),
    ("V4 rapier_version 0.35.0", {"V4"}, set(), summ(lambda d: d.update(rapier_version="0.35.0"))),
    ("V4 avx2 false", {"V4"}, set(), summ(lambda d: d["target_features"].update(avx2=False))),
    ("V4 exe sha256 swapped to the other arm's exe", {"V4"}, set(), lambda c: c.update(exe_sha256=c["pins"]["exe_sha256"][other_arm(c["arm"])])),
    ("V4 SUMMARY arm label swapped", {"V4"}, set(), lambda c: c.update(log_text=edit_summary(c["log_text"], lambda d: d.update(arm=other_arm(c["arm"]))))),
    ("V4 source hash edited", {"V4"}, set(), summ(lambda d: d["source_fnv1a64"].update(main_rs="0x0000000000000000"))),
    ("V5 num_solver_iterations 4 -> 3", {"V5"}, set(), summ(lambda d: d["config"].update(num_solver_iterations=3))),
    ("V5 counters_enabled true", {"V5"}, set(), summ(lambda d: d["config"].update(counters_enabled=True))),
    ("V5 the other config's parameters", {"V5"}, set(), set_other_config),
    ("V6 spawn hash edited", {"V6"}, set(), summ(lambda d: d["scene_identity"].update(spawn_hash="0x0000000000000001"))),
    ("V6 perturb not null", {"V6"}, set(), summ(lambda d: d.update(perturb={"box": 0, "axis": 0, "dir": 1}))),
    ("V6 first_equals_ours false", {"V6"}, set(), summ(lambda d: d["scene_identity"].update(first_equals_ours=False))),
    ("V7 final pose differs by 1 ulp", {"V7"}, set(), flip_pose),
    ("V7 SUMMARY expect_pose mismatch", {"V7"}, set(), summ(lambda d: d.update(expect_pose="mismatch: 1 vs 2 bytes, first differing body Some(0)"))),
    ("V8 CSV has 499 rows", {"V8"}, {"V3"}, csv_edit(lambda ls: ls[:-1])),
    ("V8 wall_ns 0 in row 10", {"V8"}, set(), csv_edit(row_edit(10, 1, 0))),
    ("V8 CSV carries receipt columns", {"V8"}, set(), csv_edit(lambda ls: [ls[0] + ",bp_pairs"] + [l + ",1" for l in ls[1:]])),
    ("V9 --receipt among the args", {"V9"}, set(), summ(lambda d: d["args"].append("--receipt"))),
    ("V9 the prep twin of the cell failed", {"V9"}, set(), set_twin),
]
