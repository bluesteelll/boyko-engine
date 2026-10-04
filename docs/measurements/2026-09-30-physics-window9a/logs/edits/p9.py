import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
apply('tools/window9a_run.py', [
 ("""        wave += 1
        if wave > RERUN_MAX_WAVES:
            ended = f'{RERUN_MAX_WAVES} waves spent'
            break
""",
  """        if wave >= RERUN_MAX_WAVES:
            ended = f'{RERUN_MAX_WAVES} waves spent'
            break
        wave += 1
"""),
])
