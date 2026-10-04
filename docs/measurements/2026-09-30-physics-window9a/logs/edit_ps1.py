p = 'tools/wait_idle9a.ps1'
s = open(p, encoding='utf-8').read()
old = "'ld', 'collect2', 'mingw32-make', 'cmake', 'rust-lld', 'lld')"
new = "'ld', 'collect2', 'mingw32-make', 'cmake', 'rust-lld', 'lld', 'rapier-parity-simd8', 'rapier-parity-simd4')"
assert s.count(old) == 1
s = s.replace(old, new)
hdr = "# Idle wait for window 9a (2026-09-30):"
assert s.startswith(hdr)
s = ("# 2026-09-30 (Rapier block): + 'rapier-parity-simd8', 'rapier-parity-simd4' in $build (window9a_rows.md preconditions: the idle-rule list names the\n"
     "# harness's own exes; the window's copies bin/rapier_parity_simd{8,4}_<sha8>.exe use underscores and never match). Nothing else changed.\n") + s
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
