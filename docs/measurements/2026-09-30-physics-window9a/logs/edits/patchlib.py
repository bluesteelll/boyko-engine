import sys
def apply(path, pairs):
    b = open(path, encoding='utf-8', newline='').read()
    crlf = '\r\n' in b
    for i, (old, new) in enumerate(pairs):
        n = b.count(old)
        if n != 1:
            sys.exit(f'{path}: edit #{i}: {n} matches of {old[:80]!r}')
        b = b.replace(old, new)
    open(path, 'w', encoding='utf-8', newline='').write(b)
    print(f'{path}: {len(pairs)} edits applied (crlf={crlf})')
