"""Arithmetic on the SPLIT medians (v_split.py definitions), [100,500). Output: v_derive.txt."""
import os, statistics, json
import vlib as L
import importlib.util
spec = importlib.util.spec_from_file_location('vs', os.path.join(L.HERE, 'v_split.py'))
OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
import io, contextlib
with contextlib.redirect_stdout(io.StringIO()):
    vs = importlib.util.module_from_spec(spec); spec.loader.exec_module(vs)
win = '100..500'
m = {}
for W in (8, 16):
    ds = [vs.perwave(p, win) for p in vs.sel('SPLIT-J-T-a', W)]
    m[W] = dict((k, statistics.median(d[k] for d in ds) / 1e3) for k in ds[0])  # us
wide1 = vs.med('SPLIT-J-T-a', 1, 'wide', win)
for W in (8, 16):
    x = m[W]
    Lw = 1e3 * (vs.med('SPLIT-J-T-a', W, 'wide', win) - wide1 / W)  # us per step
    wv = 96.0
    ramp, join, imb = wv * x['ramp'], wv * x['join'], wv * x['imb']
    inw = Lw - ramp - join - imb
    x.update(Lw=Lw, ramp_s=ramp, join_s=join, imb_s=imb, inw=inw)
    P('W%d: L_wide %.1f us/step; ramp %.2f (%.1f %%) join %.2f (%.1f %%) imbalance %.2f (%.1f %%) in-wave %.1f (%.1f %%)' % (
        W, Lw, ramp, 100 * ramp / Lw, join, 100 * join / Lw, imb, 100 * imb / Lw, inw, 100 * inw / Lw))
    P('   first ramp / helped ramp %.2fx; pass ramp / other ramp %.2fx; 12 pass ramps %.2f us = %.1f %% of step ramp; 11 after first %.2f us' % (
        x['first_ramp'] / x['ramp_helped'], x['pass_ramp'] / x['other_ramp'], 12 * x['pass_ramp'], 100 * 12 * x['pass_ramp'] / ramp,
        12 * x['pass_ramp'] - x['first_ramp']))
    P('   tail = imbalance %.1f %% + join %.1f %%; ramp+tail per wave %.3f us = %.1f %% of omega %.3f us' % (
        100 * x['imb'] / x['tail'], 100 * x['join'] / x['tail'], x['ramp'] + x['tail'], 100 * (x['ramp'] + x['tail']) / (Lw / wv), Lw / wv))
    P('   S1 dispatch target = ramp + join - first ramp = %.1f us/step; np: ramp %.2f tail %.2f = imb %.2f (%.1f %%) + join %.2f' % (
        ramp + join - x['first_ramp'], x['np_ramp'], x['np_tail'], x['np_imb'], 100 * x['np_imb'] / x['np_tail'], x['np_join']))
g = lambda k: m[16][k] - m[8][k]
P('W16-W8 growth: L_wide %+.1f; in-wave %+.1f (%.1f %%), imbalance %+.1f, join %+.1f, ramp %+.1f us/step' % (
    g('Lw'), g('inw'), 100 * g('inw') / g('Lw'), g('imb_s'), g('join_s'), g('ramp_s')))
open(os.path.join(L.HERE, 'v_derive.txt'), 'w').write(chr(10).join(OUT) + chr(10))
print(chr(10).join(OUT))
