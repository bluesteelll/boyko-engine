import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
apply('tools/window9a_run.py', [
 ("""for key, e in BINS.items():
    e['sha256'] = SUMS.get(e['exe'])
""",
  """for key, e in BINS.items():
    e['sha256'] = SUMS.get(e['exe'])
    if e['sha256'] is None and (TEST or DRY):     # a rehearsal overlay may bring an exe that is not in bin/SHA256SUMS: its own pin
        e['sha256'] = e.get('sha256_pin')
"""),
])
