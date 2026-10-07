#!/usr/bin/env python3
"""Dynamic parity scenes: the cross-engine identity gate on two canonical scene dumps.

usage: compare_dumps.py A B
exit 0 when the files are equal byte for byte (prints IDENTICAL and the FNV-1a 64), 1 otherwise
(prints the first differing line, both versions, and the fields that differ), 2 unreadable.
"""
import sys

M64 = (1 << 64) - 1


def fnv1a64(data):
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & M64
    return h


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        a, b = (open(p, 'rb').read() for p in sys.argv[1:])
    except OSError as e:
        print('compare_dumps: %s' % e, file=sys.stderr)
        return 2
    if a == b:
        print('IDENTICAL 0x%016x %d bytes %d lines' % (fnv1a64(a), len(a), a.count(b'\n')))
        return 0
    la, lb = a.split(b'\n'), b.split(b'\n')
    for n, (x, y) in enumerate(zip(la, lb), 1):
        if x != y:
            fx, fy = x.split(b' '), y.split(b' ')
            diff = [i for i in range(max(len(fx), len(fy))) if i >= len(fx) or i >= len(fy) or fx[i] != fy[i]]
            print('DIFFER at line %d (fields %s)' % (n, diff))
            print('  A: %r' % x.decode('ascii', 'replace'))
            print('  B: %r' % y.decode('ascii', 'replace'))
            return 1
    print('DIFFER in length: %d vs %d lines (one is a prefix of the other)' % (len(la), len(lb)))
    return 1


if __name__ == '__main__':
    sys.exit(main())
