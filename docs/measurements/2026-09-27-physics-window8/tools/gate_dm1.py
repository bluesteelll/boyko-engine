"""Window 8 untimed DM1 gate: one windowed process at a time, validation off, every run bounded by its frame count; a
process alive past 300 s is a hang and is terminated through its own handle (never by image name).
  crash  : each exe once at 1280x720 with BOYKO_VB_BENCH_FRAMES=10 (the zone leg must end by itself and write the artifact)
  res    : dm1_A at 1920x1080, 2560x1440, 3840x2160 (10 frames): which client areas the display can host
  sample : one full-size dry sample per DM1 row kind (220 frames) for the schedule estimate: vb idle, deferred idle,
           vb edit100, vb grow150 (B)
Reads no zone or timing number: exit, the harness's "DM1 timing:" line, the libtest result, the artifact's existence.
Writes gate/dm1/<tag>/, gate/gate_dm1.json, gate/dm1_res.json, appends gate/estimates.json and gate/gate_run.log."""
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
GATE = os.path.join(W8, 'gate')
LOG = os.path.join(GATE, 'gate_run.log')
EST = os.path.join(GATE, 'estimates.json')
EXE = {'dmA': os.path.join(W8, 'bin', 'dm1_A.exe'), 'dmB': os.path.join(W8, 'bin', 'dm1_B.exe')}
ARGS = ['dm1_material_table_timing', '--exact', '--ignored', '--test-threads=1', '--nocapture']
LINE = re.compile(r'DM1 timing: path=(\w+) res=(\d+)x(\d+) edit_rows=(\d+) grow_at=(\S+) frames=(\d+)')


def log(msg):
    line = f'{time.strftime("%Y-%m-%dT%H:%M:%S")} {msg}'
    print(line, flush=True)
    with open(LOG, 'a', encoding='utf-8', newline='\n') as f:
        f.write(line + '\n')


def run(tag, key, path, res, frames, edit=0, grow=None):
    cwd = os.path.join(GATE, 'dm1', tag)
    os.makedirs(cwd, exist_ok=True)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    env.update({'BOYKO_DISABLE_VALIDATION': '1', 'BOYKO_VB_ZONE': '1', 'BOYKO_VB_BENCH_FRAMES': str(frames),
                'BOYKO_DM1_PATH': path, 'BOYKO_DM1_RES': res, 'BOYKO_DM1_EDIT_ROWS': str(edit),
                'BOYKO_PROFILE_ARTIFACT': os.path.join(cwd, 'artifact.toml')})
    if grow is not None:
        env['BOYKO_DM1_GROW_AT'] = str(grow)
    t0 = time.perf_counter()
    p = subprocess.Popen([EXE[key]] + ARGS, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    hang = False
    try:
        so, se = p.communicate(timeout=300)
    except subprocess.TimeoutExpired:
        hang = True
        p.kill()  # TerminateProcess on this Popen's own handle
        so, se = p.communicate()
    wall = round(time.perf_counter() - t0, 2)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(so)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(se)
    txt = so.decode('utf-8', 'replace') + '\n' + se.decode('utf-8', 'replace')
    m = LINE.search(txt)
    art = os.path.join(cwd, 'artifact.toml')
    rec = {'tag': tag, 'binary': key, 'path': path, 'res': res, 'frames_env': frames, 'edit_rows': edit, 'grow_at': grow,
           'pid': p.pid, 'exit': p.returncode, 'hang': hang, 'wall_s_for_schedule': wall,
           'dm1_line': m.group(0) if m else None, 'passed': bool(re.search(r'test result: ok\. 1 passed', txt)),
           'not_measured': 'NOT MEASURED' in txt, 'artifact_bytes': os.path.getsize(art) if os.path.exists(art) else None}
    m2 = re.search(r'the window came up (\d+)x(\d+)', txt)
    rec['came_up'] = f'{m2.group(1)}x{m2.group(2)}' if m2 else None
    panic = re.search(r"panicked at [^\n]*\n([^\n]*)", txt)
    rec['panic'] = panic.group(0)[:300] if panic else None
    ok = rec['exit'] == 0 and not hang and rec['passed'] and m is not None and rec['artifact_bytes']
    rec['pass'] = bool(ok)
    log(f'dm1 {tag:28s} {key} {path:8s} {res:9s} frames {frames:3d} exit {rec["exit"]} hang {hang} passed {rec["passed"]} '
        f'line {rec["dm1_line"]!r} artifact {rec["artifact_bytes"]} came_up {rec["came_up"]} {"PASS" if ok else "FAIL"}')
    return rec


if __name__ == '__main__':
    part = sys.argv[1]
    out_p = os.path.join(GATE, 'gate_dm1.json')
    recs = json.load(open(out_p, encoding='utf-8')) if os.path.exists(out_p) else []
    if part == 'crash':
        recs.append(run('crash_A_vb_1280x720', 'dmA', 'vb', '1280x720', 10))
        recs.append(run('crash_B_vb_1280x720', 'dmB', 'vb', '1280x720', 10))
    elif part == 'res':
        rr = {}
        for res in ('1920x1080', '2560x1440', '3840x2160'):
            r = run(f'res_A_vb_{res}', 'dmA', 'vb', res, 10)
            recs.append(r)
            rr[res] = {'hosted': r['pass'] and not r['not_measured'], 'came_up': r['came_up'], 'exit': r['exit']}
        json.dump(rr, open(os.path.join(GATE, 'dm1_res.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    elif part == 'sample':
        est = json.load(open(EST, encoding='utf-8')) if os.path.exists(EST) else {}
        for tag, key, path, res, edit, grow, rows in (
                ('sample_A_vb_idle', 'dmA', 'vb', '1920x1080', 0, None, ['dm-vb-{r}-idle']),
                ('sample_A_deferred_idle', 'dmA', 'deferred', '1920x1080', 0, None, ['dm-deferred-{r}-idle']),
                ('sample_A_vb_edit100', 'dmA', 'vb', '1920x1080', 100, None, ['dm-vb-1920x1080-edit100', 'dm-deferred-1920x1080-edit100']),
                ('sample_B_vb_grow150', 'dmB', 'vb', '1920x1080', 0, 150, ['dm-vb-1920x1080-grow150'])):
            r = run(tag, key, path, res, 220, edit, grow)
            recs.append(r)
            for pat in rows:
                for rs in (('1920x1080', '2560x1440', '3840x2160') if '{r}' in pat else ('',)):
                    for k in ('dmA', 'dmB'):
                        est[f'{pat.format(r=rs)}#{k}@W0'] = r['wall_s_for_schedule']
        json.dump(est, open(EST, 'w', encoding='utf-8', newline='\n'), indent=1, sort_keys=True)
    json.dump(recs, open(out_p, 'w', encoding='utf-8', newline='\n'), indent=1)
