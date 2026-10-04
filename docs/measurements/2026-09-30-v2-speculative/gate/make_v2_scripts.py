"""V2 (C6): write the `_v2` pose-gate scripts from the trunk's three L10 scripts in `l10/`: every
fixture directory re-pointed at the V2 recordings and every pinned hash replaced by the V2
recording of the same row (critique W5: the hashes too, not only the paths). The originals are
read, never edited. usage: python make_v2_scripts.py  (writes the three `*_v2.sh` beside itself)"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
FX = 'D:/wt/merge/docs/measurements/2026-09-30-v2-speculative'
SRC = ['pose_gates_sync.sh', 'runner_checks_off.sh', 'runner_checks_c3c.sh']
DIRS = [  # (assignment in the originals, the V2 directory)
    ('L9=D:/wt/merge/docs/measurements/2026-09-23-l9-contact-reuse/fixtures\n', f'L9={FX}/l9-reuse-off\n'),
    ('L9C4=D:/wt/merge/docs/measurements/2026-09-23-l9-contact-reuse/fixtures-c4\n', f'L9C4={FX}/l9-reuse-on\n'),
    ('L10=D:/wt/merge/docs/measurements/2026-09-23-l10-sleeping/fixtures\n', f'L10={FX}/l10-reuse-on\n'),
    ('L10OFF=D:/wt/merge/docs/measurements/2026-09-23-l10-sleeping/fixtures-reuse-off\n', f'L10OFF={FX}/l10-reuse-off\n'),
    ('W6=D:/wt/merge/docs/measurements/2026-09-24-physics-window6/gate/fixtures\n', f'W6={FX}/w6\n'),
]
HASHES = {  # the pre-V2 hash -> the V2 recording of the same row
    '0x30c5438bc6ad9ffa': ('w8', 'J500'),
    '0x32d5e235342b4143': ('w8', 'J500off'),
    '0x3db47fae414b655c': ('w6', 'Offp-J1000'),
    '0xcc2a5400c66eecce': ('w6', 'Son-J1000'),
    '0xb7f1e9e8f91f75ab': ('w6', 'RSOffp800'),
    '0x2a2b7926a48aab00': ('w6', 'RS800'),
}

rec = {}
for line in open(os.path.join(HERE, '..', 'logs', 'record.tsv')):
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
    assert not any(h in s for h in HASHES), f'{name}: a pre-V2 hash survived'
    s = s.replace('#!/usr/bin/env bash\n', f'#!/usr/bin/env bash\n# V2 (C6): generated from l10/{name} by make_v2_scripts.py - the fixtures and pinned\n# hashes are the V2 recordings ({FX}).\n', 1)
    dst = os.path.join(HERE, name.replace('.sh', '_v2.sh'))
    open(dst, 'w', newline='\n').write(s)
    print(f'{os.path.basename(dst)}: {n_dirs} directories, {n_hash} hash sites')
