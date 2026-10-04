p = 'tools/mkrows9a.py'
s = open(p, encoding='utf-8').read()
i = s.index("proto['own_k'] = (")
j = s.index("proto['criterion'] = (")
new = '''proto['own_k'] = ('C4-G4-kd (F3 keep/freeze, not gating) keeps the Q3 recipe\'s own K = 3 (one pass x three rounds). C4-G4 runs the letter (K = 9). '
                  + ('C4-BR runs K = 3 (block "passes" 1, the lane\'s Q5 recommendation) as a DIAGNOSTIC: NOT claim-bearing (orchestrator '
                     'ruling (b), 2026-09-30); `python -B tools/mkrows9a.py --br-k9` restores the letter.' if BR_K3 else
                     'C4-BR also runs the letter (K = 9; generated with --br-k9).'))
'''
s = s[:i] + new + s[j:]
open(p, 'w', encoding='utf-8', newline='\n').write(s)
