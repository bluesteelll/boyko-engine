import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
import lib8 as L

PROCS = [p for p in json.load(open(os.path.join(L.HERE, 'proc8.json')))['procs'] if p.get('used')]
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def sel(row, W, blocks=None, binary=None, passes=None):
    return [p for p in PROCS if p['row'] == row and p['W'] == W and (blocks is None or p['block'] in blocks)
            and (binary is None or p['binary'] == binary) and (passes is None or p['pass'] in passes)]


def C(row, W, f, blocks=None, binary=None, passes=None):
    return L.cell([f(p) for p in sel(row, W, blocks, binary, passes)])


def wall(win='0..500'):
    return lambda p: p['wall_ms'][win]


def col(name, win='100..500', scale=1e-6):
    def g(p):
        v = p['stats'][win].get(name)
        return None if v is None else v * scale
    return g


def save(name):
    open(os.path.join(L.HERE, name), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
