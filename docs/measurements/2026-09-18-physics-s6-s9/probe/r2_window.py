"""MEASUREMENT-QUEUE §9 R2 helper: mean live contact points per TIMED step, from the probe series.

Timed-step model (criterion 0.5, sample_size 20, warm-up 3 s, measurement 5 s):
  * sleeping_pipeline arms: ONE persistent pile. Setup steps 1..30 (WARM_STEPS), then the
    criterion warm-up runs W = 2^(k+1)-1 steps (doubling batches until > 3 s), then the
    measurement runs N steps (printed by criterion: "Collecting 20 samples in estimated X s
    (N iterations)"). Timed window = steps [31+W, 30+W+N]. Only the N measured steps count.
  * jolt_parity_pyramid full_step/w: EVERY sample rebuilds the world and steps 20 untimed, so
    sample j with i_j iterations times steps 21..20+i_j. Linear sampling: i_j = j*d, j=1..20,
    N = 210*d. Flat sampling: i_j = m for all 20 samples, N = 20*m.
usage:
  r2_window.py <mq-dir> sp   <sha8> <arm> <W> <N>
  r2_window.py <mq-dir> jolt <sha8> <arm> linear <d> | flat <m>
  r2_window.py <mq-dir> table
"""
import csv, sys, collections

def load(mq, kind, sha):
    path = f'{mq}/probe/{"sp" if kind == "sp" else "jolt"}_{sha}.csv'
    s = collections.defaultdict(dict)
    for r in csv.DictReader(open(path)):
        s[r['arm']][int(r['step'])] = int(r['points'])
    return s

def sp_mean(series, W, N):
    lo, hi = 31 + W, 30 + W + N
    assert hi <= max(series), f'window ends at {hi}, probe series ends at {max(series)}'
    return sum(series[s] for s in range(lo, hi + 1)) / N, (lo, hi)

def jolt_mean(series, mode, x):
    iters = [j * x for j in range(1, 21)] if mode == 'linear' else [x] * 20
    assert 20 + max(iters) <= max(series)
    tot = sum(sum(series[s] for s in range(21, 21 + i)) for i in iters)
    return tot / sum(iters), (21, 20 + max(iters))

if __name__ == '__main__':
    mq = sys.argv[1]
    if sys.argv[2] == 'sp':
        sha, arm, W, N = sys.argv[3], sys.argv[4], int(sys.argv[5]), int(sys.argv[6])
        m, win = sp_mean(load(mq, 'sp', sha)[arm], W, N)
        print(f'{sha} {arm} window {win}: mean points/step {m:.1f}')
    elif sys.argv[2] == 'jolt':
        sha, arm, mode, x = sys.argv[3], sys.argv[4], sys.argv[5], int(sys.argv[6])
        m, win = jolt_mean(load(mq, 'jolt', sha)[arm], mode, x)
        print(f'{sha} {arm} {mode} {x} steps {win}: mean points/timed step {m:.1f}')
    else:
        A, B = '08fe7b9f', '8d656ad8'
        spA, spB = load(mq, 'sp', A), load(mq, 'sp', B)
        jA, jB = load(mq, 'jolt', A), load(mq, 'jolt', B)
        for arm in ['pyramid_sleeping_off', 'pyramid_awake_sleeping_on']:
            print(f'== {arm}: timed window [31+W, 30+W+N]; mean points/step A, B, B/A (same window)')
            for W in [127, 255, 511, 1023]:
                for N in [100, 210, 420, 630, 840]:
                    a, win = sp_mean(spA[arm], W, N); b, _ = sp_mean(spB[arm], W, N)
                    print(f'  W={W:4d} N={N:3d} steps {win[0]}-{win[1]}: A {a:8.1f}  B {b:8.1f}  B/A {b/a:.4f}')
        for arm in ['full_step/1', 'full_step/4']:
            print(f'== {arm}: every sample from a fresh world, timed steps 21..20+i; mean points/timed step')
            for mode, xs in [('linear', [1, 2, 3, 4, 6, 8, 12, 16, 24]), ('flat', [3, 5, 8, 12])]:
                for x in xs:
                    a, win = jolt_mean(jA[arm], mode, x); b, _ = jolt_mean(jB[arm], mode, x)
                    print(f'  {mode:6s} {x:3d} (N={sum([j*x for j in range(1,21)]) if mode=="linear" else 20*x:4d}) steps {win[0]}-{win[1]}: A {a:8.1f}  B {b:8.1f}  B/A {b/a:.4f}')
        # sameness checks
        print('awake_sleeping_on series == full_step/1 series (steps 1..3000):',
              'A', all(spA['pyramid_awake_sleeping_on'][s] == jA['full_step/1'][s] for s in range(1, 3001)),
              'B', all(spB['pyramid_awake_sleeping_on'][s] == jB['full_step/1'][s] for s in range(1, 3001)))
        print('full_step/4 series == full_step/1 series (steps 1..3000):',
              'A', jA['full_step/4'] == jA['full_step/1'], 'B', jB['full_step/4'] == jB['full_step/1'])
