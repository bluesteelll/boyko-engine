"""Window 5 MAKE-UP for the slots the protocol dropped (original AND its one re-run both contaminated), so that a
K = 6 variant of those cells exists beside the protocol's K = 5. Labelled 'makeup' everywhere, written to
raw/makeup/runs_makeup.jsonl (NOT raw/runs.jsonl, so the protocol reduction is untouched).
Same machinery as window5_run.py (imported): idle rule (wait_idle5.ps1, 120 polls), 10-s opening receipt with the
presence snapshot, one untimed warm-up (tip J-As W=8), then each dropped slot's cell ONCE, in the slot order, 5-s
receipts between; a contaminated make-up is recorded and NOT retried; a build/lane process at any point voids the
make-up pass (idle rule, redo, up to 20 times)."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import window5_run as R  # noqa: E402

DROPPED = [('HL-D-tree', 'tip', 1), ('HL-D-allpairs', 'tip', 2), ('J-As', 'tip', 8)]


def main():
    out = os.path.join(R.RAW, 'makeup')
    os.makedirs(out, exist_ok=True)
    runs = os.path.join(out, 'runs_makeup.jsonl')
    m = json.load(open(os.path.join(R.RAW, 'manifest.json'), encoding='utf-8'))
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    sampler = R.PerfSampler(1.0)
    R.log(f'MAKEUP START for dropped slots {DROPPED}')
    status = 'complete'
    attempt = 0
    while True:
        R.log(f'makeup-a{attempt}: idle rule')
        rc = R.wait_idle()
        if rc != 0:
            status = f'STOP: idle rule not met (exit {rc})'
            break
        bad = R.verify_binaries(m)
        if bad:
            status = f'STOP: binaries changed {bad}'
            break
        pdir = os.path.join(out, f'makeup-a{attempt}')
        os.makedirs(pdir, exist_ok=True)
        receipt = R.receipt_with_presence(R.PASS_RECEIPT_S)
        R.log(f'makeup-a{attempt}: opening receipt {receipt["cpu_avg"]}% presence {receipt["presence"]}')
        voided = R.void_seen(receipt)
        if not voided:
            wrec, receipt, st = R.run_one(m, 'J-As', 'tip', 8, 'makeup', 9, attempt, -1, 0, 'warmup', pdir, receipt, env, sampler)
            R.append(runs, wrec)
            R.log(f'  makeup warm-up: {R.tag_of(wrec)}')
            voided = st == 'void'
            if st == 'abort':
                status = f'ABORT {wrec.get("aborted")}'
                break
        if not voided:
            for i, (rid, role, w) in enumerate(DROPPED, 1):
                rec, receipt, st = R.run_one(m, rid, role, w, 'makeup', 9, attempt, 0, i, 'makeup', pdir, receipt, env, sampler)
                R.append(runs, rec)
                R.log(f'  makeup [{i}] {rid}#{role} W={w}: {R.tag_of(rec)}')
                if st == 'abort':
                    status = f'ABORT {rec.get("aborted")}'
                    break
                if st == 'void':
                    voided = True
                    break
        if status != 'complete':
            break
        if not voided:
            break
        R.append(runs, {'voided_pass': True, 'block': 'makeup', 'pass': 9, 'pass_attempt': attempt, 'time': R.D.now()})
        R.log(f'makeup-a{attempt}: VOIDED; waiting for idle and redoing')
        attempt += 1
        if attempt > 20:
            status = 'STOP: voided 21 times'
            break
    sampler.close()
    R.log(f'MAKEUP END {status}')
    open(os.path.join(out, 'MAKEUP_DONE'), 'w', encoding='utf-8').write(status + '\n')


if __name__ == '__main__':
    main()
