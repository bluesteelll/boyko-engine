import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
P = 'tools/window9a_run.py'
apply(P, [
 ("RAW = os.path.join(W8, 'test', 'window') if TEST else os.path.join(W8, 'raw')\n",
  "RAW = os.path.join(W8, 'test', 'window') if TEST else os.path.join(W8, 'raw')\n"
  "if TEST and os.environ.get('W9A_TEST_RAW'):    # rehearsals only (selftest9a's end-to-end re-run cases): a scratch raw dir\n"
  "    RAW = os.environ['W9A_TEST_RAW']\n"),
 ("""    if TEST and attempt_no < INJECT.get((rid, key, w, rnd), 0):
        rec['contaminated'] = True
        rec['injected_hot'] = True
""",
  """    if TEST and INJECT:
        # With an injection active the rehearsal is deterministic: contamination is EXACTLY the injected pattern (the real
        # receipts of a shared machine are kept as contaminated_real, not used).
        rec['contaminated_real'] = rec.get('contaminated')
        rec['contaminated'] = attempt_no < INJECT.get((rid, key, w, rnd), 0)
        rec['injected_hot'] = rec['contaminated']
"""),
])
