p = 'run_window9a.sh'
s = open(p, encoding='utf-8').read()
old = """# PHYSICS WINDOW 9a (2026-09-30; PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a): C4-AB (with the in-block Jolt
# 5.6 row) -> C4-G5 -> C4-BR -> C4-G4 -> C4-G4-kd (blocks run in the priority order of rows9a.json, which is this one; the
# lane's own order BR, G4, G4-kd, AB, G5 is `python -B tools/mkrows9a.py --lane-order`; reserved blocks from
# rows9a.extra.json, e.g. Rapier, sort in by their priority), alone on an idle machine."""
new = """# PHYSICS WINDOW 9a (2026-09-30; PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a): C4-AB (with the in-block Jolt
# 5.6 row) -> C4-RAPIER (Rapier 0.36, rows9a.extra.json, priority 1.5) -> C4-G5 -> C4-BR (K 3, DIAGNOSTIC ONLY, not
# claim-bearing) -> C4-G4 -> C4-G4-kd (blocks run in the priority order of rows9a.json + rows9a.extra.json; the lane's own
# order BR, G4, G4-kd, AB, G5 is `python -B tools/mkrows9a.py --lane-order`), alone on an idle machine."""
assert s.count(old) == 1
s = s.replace(old, new)
old2 = "#   --blocks a,b               only these blocks (C4-AB, C4-G5, C4-BR, C4-G4, C4-G4-kd, + any rows9a.extra.json block)"
new2 = "#   --blocks a,b               only these blocks (C4-AB, C4-RAPIER, C4-G5, C4-BR, C4-G4, C4-G4-kd)"
assert s.count(old2) == 1
s = s.replace(old2, new2)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
