"""V2 (C6): write the `_v2` pose-gate scripts from the trunk's three L10 scripts in `l10/`: every
fixture directory re-pointed at the V2 recordings and every pinned hash replaced by the V2
recording of the same row (critique W5: the hashes too, not only the paths). The originals are
read, never edited. Since lane SL (ruling 39) both sets live in docs/measurements/2026-10-11-sl-sleeping:
l10/ reads its d0/ mirror, and this script maps it onto its v2/ mirror and record log. usage: python make_v2_scripts.py  (writes the three `*_v2.sh` beside itself)
Every fixture directory is `$ROOT/<repo-relative path>`: the scripts resolve ROOT themselves (their
gate-root block), so no checkout's absolute path is written into them."""
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
FX = 'docs/measurements/2026-10-11-sl-sleeping/v2'  # repo-relative: lane SL's mirror of V2's fixtures (ruling 39)
SLD0 = 'docs/measurements/2026-10-11-sl-sleeping/d0'  # the d0 mirror l10/ reads; never in a _v2 script
SRC = ['pose_gates_sync.sh', 'runner_checks_off.sh', 'runner_checks_c3c.sh']
DIRS = [  # (assignment in the originals, the V2 directory)
    (f'L9=$ROOT/{SLD0}/l9-fixtures\n', f'L9=$ROOT/{FX}/l9-reuse-off\n'),
    (f'L9C4=$ROOT/{SLD0}/l9-fixtures-c4\n', f'L9C4=$ROOT/{FX}/l9-reuse-on\n'),
    (f'L10=$ROOT/{SLD0}/l10-fixtures\n', f'L10=$ROOT/{FX}/l10-reuse-on\n'),
    (f'L10OFF=$ROOT/{SLD0}/l10-fixtures-reuse-off\n', f'L10OFF=$ROOT/{FX}/l10-reuse-off\n'),
    (f'W6=$ROOT/{SLD0}/w6-fixtures\n', f'W6=$ROOT/{FX}/w6\n'),
]
HASHES = {  # the d0 hash pinned in l10/ -> the V2 recording of the same row (lane SL: both sets SL-1's)
    '0x30c5438bc6ad9ffa': ('v2/w8', 'J500'),
    '0x32d5e235342b4143': ('v2/w8', 'J500off'),
    '0x74c14f3a08e24421': ('v2/w6', 'Offp-J1000'),
    '0x13d95ddbd8f6907e': ('v2/w6', 'Son-J1000'),
    '0xa42b663994f7b36d': ('v2/w6', 'RSOffp800'),
    '0xd70e51f6ae4dd2b8': ('v2/w6', 'RS800'),
}

rec = {}
for line in open(os.path.join(HERE, '..', '..', '2026-10-11-sl-sleeping', 'logs', 'record.tsv')):
    d, name, _exit, h = line.rstrip('\n').split('\t')[:4]
    rec[(d, name)] = h
for name in SRC:
    s = open(os.path.join(HERE, 'l10', name)).read()
    n_dirs = 0
    for old, new in DIRS:
        if old in s:
            s = s.replace(old, new)
            n_dirs += 1
    n_hash = 0
    for old, key in HASHES.items():
        c = s.count(old)
        if c:
            s = s.replace(old, rec[key])
            n_hash += c
    assert 'docs/measurements/2026-09-2' not in s, f'{name}: a pre-V2 fixture path survived'
    assert SLD0 not in s, f'{name}: an SL d0 fixture path survived'
    assert not any(h in s for h in HASHES), f'{name}: a pre-V2 hash survived'
    assert not re.search(r'[A-Za-z]:/wt/', s), f'{name}: a checkout path survived'
    s = s.replace('#!/usr/bin/env bash\n', f'#!/usr/bin/env bash\n# V2 (C6): generated from l10/{name} by make_v2_scripts.py - the fixtures and pinned\n# hashes are the V2 recordings ($ROOT/{FX}).\n', 1)
    dst = os.path.join(HERE, name.replace('.sh', '_v2.sh'))
    open(dst, 'w', newline='\n').write(s)
    print(f'{os.path.basename(dst)}: {n_dirs} directories, {n_hash} hash sites')
