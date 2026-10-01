p = '../tools/mkrows9a.py'
s = open(p, encoding='utf-8').read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:80])
    s = s.replace(old, new)


rep('''  python -B tools/mkrows9a.py            # writes rows9a.json (C4-BR at the letter, K 9)
  python -B tools/mkrows9a.py --br-k3    # the lane's Q5 recommendation: C4-BR one pass x three rounds (K 3)
''', '''  python -B tools/mkrows9a.py            # writes rows9a.json: C4-BR at K 3 (one pass x three rounds), DIAGNOSTIC ONLY, NOT claim-bearing
                                         # (orchestrator ruling (b), 2026-09-30, with the Rapier block; the lane's Q5 recommendation)
  python -B tools/mkrows9a.py --br-k9    # C4-BR at the letter of ruling 1 (K 9 = 3 passes x 3 rounds); the pre-2026-09-30 default
''')
rep("BR_K3 = '--br-k3' in sys.argv\n",
    "BR_K3 = '--br-k9' not in sys.argv      # 2026-09-30 ruling (b): K 3 is the default; --br-k3 (the old spelling) is accepted and means the default\n")
rep("""'the letter (K = 9); C4-BR may be cut to K = 3 by setting its block "passes" to 1 (the lane\\'s Q5 recommendation).')""",
    """'the letter (K = 9); C4-BR runs K = 3 (block "passes" 1, the lane\\'s Q5 recommendation) as a DIAGNOSTIC: not claim-bearing '
                  '(orchestrator ruling (b), 2026-09-30) - `python -B tools/mkrows9a.py --br-k9` restores the letter.')
if not BR_K3:
    proto['own_k'] = proto['own_k'].replace('C4-BR runs K = 3 (block "passes" 1, the lane\\'s Q5 recommendation) as a DIAGNOSTIC: not claim-bearing '
                                            '(orchestrator ruling (b), 2026-09-30) - `python -B tools/mkrows9a.py --br-k9` restores the letter.',
                                            'C4-BR is generated at the letter here (--br-k9): K 9.')""")
rep("""     'note': 'the all_pairs bracket (M9 verdict b: placement only): g4r7, g4r8b, g4rT adjacent in every round, one process per exe per '
             'round, CRITERION_HOME per process. K 9 (the letter); passes 1 = the lane\\'s Q5 recommendation (K 3, saves ~30 min)'},""",
    """     'note': ('the all_pairs bracket (M9 verdict b: placement only): g4r7, g4r8b, g4rT adjacent in every round, one process per exe per '
              'round, CRITERION_HOME per process. ' + ('K 3 = 1 pass x 3 rounds: DIAGNOSTIC ONLY, NOT claim-bearing (orchestrator ruling (b), '
              '2026-09-30; the lane\\'s Q5 recommendation): the analysis reads it as a bracket receipt, never as a K 9 claim, and no gate '
              'of this window rests on it' if BR_K3 else 'K 9 (the letter of ruling 1), --br-k9'))},""")
rep("""rows.append(br)
""", """if BR_K3:
    br['note'] = ('K 3 (1 pass x 3 rounds): DIAGNOSTIC ONLY, NOT claim-bearing (orchestrator ruling (b), 2026-09-30); ' + (br.get('note') or '')).strip()
rows.append(br)
""")
rep("""dev('C4-JOLT is not a block""", """if BR_K3:
    dev('C4-BR RUNS AT K 3 (block passes 1 x rounds 3), DIAGNOSTIC ONLY, NOT claim-bearing: orchestrator ruling (b) of 2026-09-30 (the lane\\'s Q5 recommendation; '
        'saves ~102 min). The letter of ruling 1 (K 9) is `python -B tools/mkrows9a.py --br-k9`; the block and its row carry the mark in their notes')
dev('C4-JOLT is not a block""")
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
