p = 'logs/mutate_rapier.py'
s = open(p, encoding='utf-8').read()
old = "def run(driver):\n    a = subprocess.run([sys.executable, '-B', os.path.join(W9, 'tools', 'selftest9a.py'), '--no-e2e', '--driver', driver], capture_output=True, text=True, cwd=W9, timeout=900)"
new = ("def run(driver, e2e=False):\n    a = subprocess.run([sys.executable, '-B', os.path.join(W9, 'tools', 'selftest9a.py'), *([] if e2e else ['--no-e2e']), '--driver', driver],\n"
       "                       capture_output=True, text=True, cwd=W9, timeout=1800)")
assert s.count(old) == 1
s = s.replace(old, new)
old2 = "def run(driver, e2e=False)"
# e2e-only mutations: they change the control flow of run_pass / run_one, which only an end-to-end rehearsal sees
old3 = "\n\ndef run(driver"
add = """

# Mutations only the END-TO-END selftest cases can see (the control flow of run_pass / run_one): run with the full selftest.
M_E2E = [
    ('run_pass runs only the first warm-up of a block', "    for wr, wk, ww in warmups_of(block):\n        ensure_time(est_s(wr, wk, ww) + RECEIPT_S, f'{ptag} warm-up {wr}#{wk}')",
     "    for wr, wk, ww in warmups_of(block)[:1]:\n        ensure_time(est_s(wr, wk, ww) + RECEIPT_S, f'{ptag} warm-up {wr}#{wk}')"),
    ('run_one never applies validate_rapier to a real process', "        rec['invalid'] = validate_rapier(rec, row, w)", "        rec['invalid'] = []"),
    ('run_one applies the RUNNER validator to a rapier record', "        rec['invalid'] = validate_rapier(rec, row, w)", "        rec['invalid'] = validate_runner(rec, row, w)"),
]
"""
assert s.count(old3) == 1
s = s.replace(old3, add + old3, 1)
old4 = "print(f'mutation proof: {caught} of {len(M)} mutations caught by at least one gate')"
new4 = """for name, old, new in M_E2E:
    n = src.count(old)
    if n != 1:
        print(f'SKIP {name}: the target text occurs {n} times')
        continue
    open(MUT, 'w', encoding='utf-8', newline='\n').write(src.replace(old, new))
    r = run(MUT, e2e=True)
    hit = r[0] != 0 or r[3] != 0
    caught += hit
    print(f'{"CAUGHT" if hit else "NOT CAUGHT":10s} [e2e] {name}: selftest exit {r[0]} ({r[1]} FAIL), xcheck exit {r[3]} ({r[4]} disagreeing) {(r[2][0][:150] if r[2] else "")}')
if os.path.exists(MUT):
    os.remove(MUT)
print(f'mutation proof: {caught} of {len(M) + len(M_E2E)} mutations caught by at least one gate')"""
assert s.count(old4) == 1
s = s.replace(old4, new4)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
