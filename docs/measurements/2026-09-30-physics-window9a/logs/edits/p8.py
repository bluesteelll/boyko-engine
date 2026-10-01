import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
apply('tools/window9a_run.py', [
 ("""    allowance_s = RERUN_STAGE_FRAC * rounds * block_estimate(block)['per_round_s']
""",
  """    allowance_s = RERUN_STAGE_FRAC * rounds * block_estimate(block)['per_round_s']
    if TEST:   # a rehearsal has no meaningful wall estimate: unbounded unless W9A_TEST_ALLOWANCE_S names one (the allowance case)
        allowance_s = float(os.environ['W9A_TEST_ALLOWANCE_S']) if os.environ.get('W9A_TEST_ALLOWANCE_S') else float('inf')
"""),
 ("            if not TEST and time.time() - stage_t0 + est_s(rid, key, w) + RECEIPT_S > allowance_s:\n",
  "            if time.time() - stage_t0 + est_s(rid, key, w) + RECEIPT_S > allowance_s:\n"),
])
