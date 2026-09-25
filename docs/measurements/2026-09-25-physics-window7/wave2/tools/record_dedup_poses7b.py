"""Record step (window 7 wave 2): keep one pose.bin per distinct pose under <record>/raw (the first in path order),
delete the byte-identical rest, and print the table (pose_hash from stdout.txt, fixture, sha256 prefix, count, kept
dir). The fixtures this wave asserted against are window 7's committed files (<record>/../gate/fixtures/*.pose);
every distinct pose must equal one of them byte for byte, or nothing is deleted.

usage: python -B record_dedup_poses7b.py <wave2 record dir>
"""
import glob
import hashlib
import os
import re
import sys

REC = os.path.abspath(sys.argv[1])
FIXD = os.path.join(os.path.dirname(REC), 'gate', 'fixtures')


def sha(p):
    return hashlib.sha256(open(p, 'rb').read()).hexdigest()


files = sorted(glob.glob(os.path.join(REC, 'raw', '**', 'pose.bin'), recursive=True))
by = {}
for f in files:
    by.setdefault(sha(f), []).append(f)
fx = {}
for p in glob.glob(os.path.join(FIXD, '*.pose')):
    fx.setdefault(sha(p), []).append(os.path.basename(p))


def pose_hash(d):
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    m = re.search(r'"pose_hash":\s*"(0x[0-9a-f]+)"', so) or re.search(r'pose_hash (0x[0-9a-f]+)', so)
    return m.group(1) if m else None


print(f'{len(files)} pose.bin files, {len(by)} distinct')
bad = [dg for dg in by if dg not in fx]
if bad:
    sys.exit(f'distinct poses with no committed fixture: {bad}; nothing deleted')
removed = 0
for dg, fl in sorted(by.items(), key=lambda kv: kv[1][0]):
    hs = sorted({pose_hash(os.path.dirname(f)) for f in fl})
    kept = os.path.relpath(os.path.dirname(fl[0]), REC).replace(os.sep, '/')
    print(f'| `{", ".join(hs)}` | {", ".join(fx[dg])} | `{dg[:8]}` | {len(fl)} | `{kept}/` |')
    for f in fl[1:]:
        os.remove(f)
        removed += 1
print('removed', removed)
