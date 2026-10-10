"""Starts window.py as soon as wait_idle.ps1 logs '# IDLE'; exits without timing on '# TIMEOUT' or '# STOP'."""
import os
import subprocess
import sys
import time

P0 = r'(session scratch)/p0'
LOG = os.path.join(P0, 'wait_log.txt')
deadline = time.time() + 4.5 * 3600
while time.time() < deadline:
    text = open(LOG, 'rb').read().decode('utf-8', errors='replace')
    if '# IDLE' in text:
        break
    if '# TIMEOUT' in text or '# STOP' in text:
        print('launcher: the machine never became idle (or D: ran low); the window was NOT started', flush=True)
        sys.exit(1)
    time.sleep(5)
else:
    print('launcher: gave up waiting for the idle poller', flush=True)
    sys.exit(1)
print('launcher: idle reached; starting window.py', flush=True)
os.makedirs(os.path.join(P0, 'raw'), exist_ok=True)
with open(os.path.join(P0, 'raw', 'window_stdout.log'), 'wb') as out:
    rc = subprocess.run([sys.executable, os.path.join(P0, 'window.py')], stdout=out, stderr=subprocess.STDOUT).returncode
print(f'launcher: window.py exit {rc}', flush=True)
sys.exit(rc)
