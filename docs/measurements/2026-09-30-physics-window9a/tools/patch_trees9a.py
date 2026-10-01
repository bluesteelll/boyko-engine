"""Window 9a: the ONE source edit, byte-exact (the exported trees are CRLF under core.autocrlf=true; a line-based editor that
rewrites endings would widen the A/B). Found by content, never by line number. Only the g4rT tree is edited: TIP and PARENT
are unmodified exports (window 8b's parent.patch, the S4-off edit, is NOT used in 9a: PARENT is the literal trunk)."""
import sys
TR = 'D:/wt/_targets/w9a-trees'
EDITS = [
    (f'{TR}/g4rT/crates/boyko_physics/benches/broadphase.rs',
     b'const G4_SIZES: [usize; 7] = [17, 64, 128, 256, 1_000, 10_000, 100_000];',
     b'const G4_SIZES: [usize; 28] = [17, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, '
     b'160, 168, 176, 184, 192, 208, 224, 240, 256, 1_000];'),
]
for path, old, new in EDITS:
    b = open(path, 'rb').read()
    n = b.count(old)
    if n != 1:
        sys.exit(f'{path}: {n} matches of {old!r}')
    open(path, 'wb').write(b.replace(old, new))
    print(f'patched {path}')
