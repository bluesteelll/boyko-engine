"""Window 9a C4-CGU (2026-10-01): where do two PE images differ? Section-by-section byte comparison (name, raw size, differing bytes,
sha256 of the raw section). Used to show that a control build of the tip from the NEW export path equals the pinned tip exe in `.text`
(the code) and differs only in link identity (28 bytes in all, measured 2026-10-01: the PE TimeDateStamp 3 B, its 3 copies in the debug directory 9 B, the CodeView GUID 16 B);
and that the cgu1 exe really has different code.
  python -B tools/pe_diff9a.py <a.exe> <b.exe>"""
import hashlib
import struct
import sys


def sections(path):
    b = open(path, 'rb').read()
    pe = struct.unpack_from('<I', b, 0x3C)[0]
    assert b[pe:pe + 4] == b'PE\0\0', path
    nsec = struct.unpack_from('<H', b, pe + 6)[0]
    opt = struct.unpack_from('<H', b, pe + 20)[0]
    off = pe + 24 + opt
    out = []
    for i in range(nsec):
        name, vsize, va, rsize, rptr = struct.unpack_from('<8sIIII', b, off + 40 * i)
        out.append((name.rstrip(b'\0').decode(), vsize, rsize, rptr))
    return b, out


def main(a, bpath):
    ba, sa = sections(a)
    bb, sb = sections(bpath)
    print(f'{"section":10s} {"rawA":>9s} {"rawB":>9s} {"diff bytes":>10s}  sha256(A)[:12] sha256(B)[:12]')
    for (na, va, ra, pa), (nb, vb, rb, pb) in zip(sa, sb):
        x, y = ba[pa:pa + ra], bb[pb:pb + rb]
        n = min(len(x), len(y))
        d = sum(1 for i in range(n) if x[i] != y[i]) + abs(len(x) - len(y))
        print(f'{na:10s} {ra:9d} {rb:9d} {d:10d}  {hashlib.sha256(x).hexdigest()[:12]} {hashlib.sha256(y).hexdigest()[:12]}' + ('' if na == nb else f'  (B: {nb})'))
    print(f'file sizes {len(ba)} {len(bb)}; sections {len(sa)} {len(sb)}')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
