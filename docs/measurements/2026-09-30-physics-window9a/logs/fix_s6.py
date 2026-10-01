p = 'logs/selftest_s6.txt'
s = open(p, encoding='utf-8').read()
a = """    case('V4 the exe is gone (sha256 MISSING)', [] if False else exactly(
        (lambda p: (R.BINS['rs8'].update(path=p), judge(rrec(*A), A[0], A[2]))[1])(os.path.join(TMPR, 'no_such.exe')), ['V4']), False)
    R.BINS['rs8']['path'] = sv_path
"""
b = """    R.BINS['rs8']['path'] = os.path.join(TMPR, 'no_such.exe')
    case('V4 the exe is gone (sha256 MISSING)', exactly(judge(rrec(*A), A[0], A[2]), ['V4']), False)
    R.BINS['rs8']['path'] = sv_path
"""
assert s.count(a) == 1
s = s.replace(a, b)
a2 = """    case('nothing passes because files are absent: stdout, run.csv and pose.bin all missing',
         [] if {'V2', 'V3', 'V7', 'V8'} <= set(rules_of(judge(rrec(*A, drop=('stdout.txt', 'run.csv', 'pose.bin')), A[0], A[2]))) else ['green'], True)
"""
b2 = """    gone = rules_of(judge(rrec(*A, drop=('stdout.txt', 'run.csv', 'pose.bin')), A[0], A[2]))
    case('nothing passes because files are absent: stdout, run.csv and pose.bin all missing turn V2, V3, V7 and V8 red',
         [] if {'V2', 'V3', 'V7', 'V8'} <= set(gone) else [f'only {gone}'], False)
"""
assert s.count(a2) == 1
s = s.replace(a2, b2)
open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ok')
