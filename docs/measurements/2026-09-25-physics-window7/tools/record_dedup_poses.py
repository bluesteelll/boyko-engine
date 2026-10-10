"""Record step: keep one pose.bin per distinct pose under <record>/raw (the first in path order), delete the
byte-identical rest, and print the table (sha256 prefix, pose_hash from stdout.txt, count, fixture, kept dir)."""
import glob
import hashlib
import os
import re
import sys

REC = sys.argv[1]
os.chdir(REC)
files = sorted(glob.glob('raw/**/pose.bin', recursive=True))
by = {}
for f in files:
    by.setdefault(hashlib.sha256(open(f, 'rb').read()).hexdigest(), []).append(f)
fx = {}
for p in glob.glob('gate/fixtures/*.pose'):
    fx.setdefault(hashlib.sha256(open(p, 'rb').read()).hexdigest(), []).append(os.path.basename(p))


def pose_hash(d):
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    m = re.search(r'pose_hash (0x[0-9a-f]+)', so)
    return m.group(1) if m else None


print(f'{len(files)} pose.bin files, {len(by)} distinct')
removed = 0
for dg, fl in sorted(by.items(), key=lambda kv: kv[1][0]):
    hs = sorted({pose_hash(os.path.dirname(f)) for f in fl})
    print(f'| `{hs}` | {fx.get(dg)} | `{dg[:8]}` | {len(fl)} | `{os.path.dirname(fl[0]).replace(os.sep, "/")}/` |')
    for f in fl[1:]:
        os.remove(f)
        removed += 1
print('removed', removed)
