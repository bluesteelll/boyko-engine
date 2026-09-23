"""Step A of the window: poll every 60 s for lanes_done.flag, log each poll, give up after 5 h.
Exit 0 = flag seen; exit 2 = 5 h elapsed without the flag."""
import datetime
import os
import subprocess
import sys
import time

W3 = r'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win3'
FLAG = os.path.join(W3, 'lanes_done.flag')
LOG = os.path.join(W3, 'wait_log.txt')
BUILD = ('cargo', 'rustc', 'link', 'lld-link', 'dxc', 'clippy-driver', 'cl', 'msbuild')
DEADLINE_S = 5 * 3600


def now():
    return datetime.datetime.now().astimezone().isoformat(timespec='seconds')


def log(msg):
    with open(LOG, 'a', encoding='utf-8') as f:
        f.write(f'{now()} {msg}\n')


def build_procs():
    ps = ('Get-Process | Where-Object { $_.Name -in @(' + ','.join(f"'{b}'" for b in BUILD) + ') } '
          '| ForEach-Object { "{0}({1})" -f $_.Name, $_.Id }')
    p = subprocess.run(['powershell.exe', '-NoProfile', '-Command', ps], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return [l.strip() for l in p.stdout.decode('utf-8', 'replace').splitlines() if l.strip()]


def heads():
    out = []
    for wt in ('D:/wt/l5np', 'D:/wt/lighttable'):
        p = subprocess.run(['git', '-C', wt, 'rev-parse', '--short=8', 'HEAD'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        out.append(f'{os.path.basename(wt)}={p.stdout.decode("utf-8", "replace").strip() or "?"}')
    return ' '.join(out)


t0 = time.time()
log(f'# wait_flag.py start; flag {FLAG}; poll 60 s; deadline {DEADLINE_S} s')
i = 0
while True:
    i += 1
    if os.path.isfile(FLAG):
        content = open(FLAG, 'rb').read().decode('utf-8', 'replace').strip()
        log(f'poll {i:3d} FLAG SEEN: {content!r}; {heads()}')
        print(f'FLAG {content}', flush=True)
        sys.exit(0)
    bp = build_procs()
    log(f'poll {i:3d} no flag; build_procs={len(bp)} [{",".join(bp)}]; {heads()}; elapsed {int(time.time() - t0)} s')
    if time.time() - t0 >= DEADLINE_S:
        log(f'# 5 h elapsed without the flag at {now()}; continuing under the idle rule')
        print('NOFLAG', flush=True)
        sys.exit(2)
    time.sleep(60)
