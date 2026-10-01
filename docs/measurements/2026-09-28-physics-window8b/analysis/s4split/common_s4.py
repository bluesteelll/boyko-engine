import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_s4 as L

PROCS = [p for p in json.load(open(os.path.join(L.HERE, 'proc_s4split.json')))['procs'] if p.get('used')]
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def sel(row, W, block, binary=None, passes=None):
    return [p for p in PROCS if p['row'] == row and p['W'] == W and p['block'] == block
            and (binary is None or p['binary'] == binary) and (passes is None or p['pass'] in passes)]


def C(row, W, block, f, binary=None, passes=None):
    return L.cell([f(p) for p in sel(row, W, block, binary, passes)])


def wall(win='0..500'):
    return lambda p: p['wall_ms'][win]


def col(name, win='100..500', scale=1e-6):
    def g(p):
        v = p['stats'][win].get(name)
        return None if v is None else v * scale
    return g


def judge(rowA, binA, rowB, binB, W, block, f, direction, rowA_W=None):
    """B vs A under ruling 1: pooled K=9 and each pass K=3. Returns dict with cells, cmps, verdict."""
    a = C(rowA, rowA_W or W, block, f, binA)
    b = C(rowB, W, block, f, binB)
    pooled = L.cmp_(a, b)
    per = []
    for ps in (0, 1, 2):
        ca = C(rowA, rowA_W or W, block, f, binA, passes=(ps,))
        cb = C(rowB, W, block, f, binB, passes=(ps,))
        per.append(dict(L.cmp_(ca, cb), pass_=ps, a=ca, b=cb))
    v, strong = L.ruling1(pooled, per, direction)
    return {'a': a, 'b': b, 'pooled': pooled, 'per': per, 'verdict': v, 'strong': strong}


def fj(j, d=4, unit='ms'):
    s = []
    s.append('  A %s' % L.fc(j['a'], d))
    s.append('  B %s' % L.fc(j['b'], d))
    s.append('  pooled B/A %s' % L.fcmp(j['pooled']))
    for c in j['per']:
        s.append('  pass %d (K=%d/%d): A %.4f B %.4f  B/A %s' % (c['pass_'], c['KA'], c['KB'], c['A'], c['B'],
                                                               L.fcmp(c)))
    s.append('  -> %s%s' % (j['verdict'], ' (STRONG)' if j['strong'] else ''))
    return '\n'.join(s)


def save(name):
    open(os.path.join(L.HERE, name), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
