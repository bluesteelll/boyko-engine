import csv, sys, collections
MQ = sys.argv[1]
def load(path):
    s = collections.defaultdict(dict)
    for r in csv.DictReader(open(path)):
        s[r['arm']][int(r['step'])] = (int(r['manifolds_live']), int(r['points']))
    return s
A = {**load(f'{MQ}/probe/sp_08fe7b9f.csv'), **load(f'{MQ}/probe/jolt_08fe7b9f.csv')}
B = {**load(f'{MQ}/probe/sp_8d656ad8.csv'), **load(f'{MQ}/probe/jolt_8d656ad8.csv')}
for arm in ['pyramid_sleeping_off', 'pyramid_awake_sleeping_on', 'full_step/1', 'full_step/4']:
    print(f'== {arm}   step: A(live manifolds/points)  B(...)  B/A points')
    n = max(A[arm])
    for st in [1, 20, 21, 30, 31, 50, 100, 200, 286, 400, 600, 705, 1000, 1500, 2000, 3000, 4000, 5000]:
        if st > n: continue
        a, b = A[arm][st], B[arm][st]
        print(f'  {st:5d}: {a[0]:5d}/{a[1]:6d}  {b[0]:5d}/{b[1]:6d}  {b[1]/a[1] if a[1] else float("nan"):.4f}')
same = all(A['full_step/1'][s] == A['full_step/4'][s] for s in A['full_step/1'])
sameB = all(B['full_step/1'][s] == B['full_step/4'][s] for s in B['full_step/1'])
print('full_step/4 series == full_step/1 series: A', same, ' B', sameB)
