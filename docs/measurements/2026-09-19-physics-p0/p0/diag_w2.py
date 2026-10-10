"""DIAGNOSTIC, not part of the window and never a quoted measurement: why A/A0 at W=2 voided.

In all three window passes J-A-d2 at W=2 ran ~2 % faster than J-A-d1 (and than J-A-a), uniformly
over the run. This replays the window's W=2 group twice, exactly as the ascending passes order it
(JOLT-T, JOLT56-T, J-A-d1, J-A-a, J-A-d2), then runs six consecutive disarmed J-A at W=2, with the
same binaries, the same arguments, the driver's receipts and the same quiet rule (window.do_run).
Output: raw/diag_w2/ (its own runs file; nothing is added to raw/runs.jsonl).
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.pop('P0_WINDOW_TEST', None)
import window as Wn  # noqa: E402
D = Wn.D

OUT = os.path.join(Wn.RAW, 'diag_w2')


def main():
    os.makedirs(OUT, exist_ok=True)
    m = json.load(open(Wn.MANIFEST, encoding='utf-8'))
    for role, e in list(m['boyko'].items()) + list(m['jolt'].items()):
        if D.sha256(e['exe']) != e['sha256']:
            sys.exit(f'{role} changed')
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    seqs = [('JOLT-T', 'rep1'), ('JOLT56-T', 'rep1'), ('J-A-d1', 'rep1'), ('J-A-a', 'rep1'), ('J-A-d2', 'rep1'),
            ('JOLT-T', 'rep2'), ('JOLT56-T', 'rep2'), ('J-A-d1', 'rep2'), ('J-A-a', 'rep2'), ('J-A-d2', 'rep2')] + \
           [('J-A-d1', f'consec{i}') for i in range(1, 7)]
    ctx = {}
    receipt = D.load_receipt(Wn.RECEIPT_WINDOW)
    path = os.path.join(OUT, 'diag_runs.jsonl')
    for i, (rid, tag) in enumerate(seqs):
        row = D.ROWS[rid]
        rec, receipt, abort = Wn.do_run(m, row, 2, i, 'original', 900 + (0 if tag.startswith('rep') else 1), OUT, ctx,
                                        receipt, env)
        rec['diag_tag'] = tag
        with open(path, 'a', encoding='utf-8') as f:
            f.write(json.dumps(rec) + '\n')
        if abort:
            print('ABORT: machine busy', flush=True)
            return
        if rec['engine'] == 'boyko':
            c = D.load_csv(rec['csv'])
            mean_ns = sum(c['wall_ns'][0:500]) / 500
        else:
            c = D.load_csv(rec['per_frame'])
            mean_ns = sum(x * 1e6 for x in c['Time (ms)']) / len(c['Time (ms)'])
        print(f'{i:2d} {tag:8s} {rid:8s} mean {mean_ns / 1e6:.4f} ms rb {rec["receipt_before"]["cpu_avg"]}% '
              f'ra {rec["receipt_after"]["cpu_avg"]}% exit {rec["exit"]}', flush=True)


if __name__ == '__main__':
    main()
