"""Arithmetic on the S4 cells (no claim): wall/span realisation, gain minus its own pooled i-bar, fill share,
P-c speedup, gain against the bar and the predictions."""
from common_s4 import *
import pickle

R = pickle.load(open(os.path.join(L.HERE, 'q_s4.pkl'), 'rb'))
for win in ('0..500', '100..500'):
    w8 = R[str((win, 'JT', 8))]['pooled']
    sp = R[str((win, 'span', 8))]['pooled']
    gw, gs = -w8['delta'], -sp['delta']
    P('[%s) W8 wall gain %.4f ms, span gain %.4f ms, wall/span %.3f; gain minus pooled i-bar (abs): wall %.4f, span %.4f; '
      'gain / 0.060 bar: wall %.2fx, span %.2fx; wall gain vs window-8 prediction 0.19-0.22: %+.4f below the low end' % (
          win, gw, gs, gw / gs, gw - w8['bar_i'] * w8['A'], gs - sp['bar_i'] * sp['A'], gw / 0.06, gs / 0.06, 0.19 - gw))
    for W in (8, 16):
        pa = C('S4-JT-a', W, 'S4-AB', col('phys_sb_pc_ns', win), 'parent')['median']
        pt = C('S4-JT-a', W, 'S4-AB', col('phys_sb_pc_ns', win), 'tip')['median']
        sa = C('S4-JT-a', W, 'S4-AB', col('phys_solve_build_ns', win), 'parent')['median']
        P('   W%d P-c PARENT %.4f TIP %.4f (speedup %.2fx, gain %.4f); PARENT fill share P-c/setup %.3f' % (
            W, pa, pt, pa / pt, pa - pt, pa / sa))
save('q_s4_arith.txt')
