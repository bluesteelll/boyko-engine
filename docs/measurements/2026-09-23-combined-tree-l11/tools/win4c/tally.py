import re,sys
lines=open(sys.argv[1],encoding='utf-8',errors='replace').read().splitlines()
cur='?'; rows=[]; bench=[]
for l in lines:
    m=re.match(r'\s+Running (\S+) \(',l)
    if m:
        cur=m.group(1)
        if cur.startswith('benches') and cur not in bench: bench.append(cur)
    m=re.match(r'^running (\d+) tests?',l)
    if m: rows.append([cur,int(m.group(1)),None])
    m=re.match(r'^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored',l)
    if m and rows and rows[-1][2] is None: rows[-1][2]=(m.group(1),int(m.group(2)),int(m.group(3)),int(m.group(4)))
P=F=I=0; zero=[]
for t,n,r in rows:
    if r is None: print("NO RESULT LINE:",t); continue
    P+=r[1];F+=r[2];I+=r[3]
    if n==0: zero.append(t)
    print(f"{t:58s} running {n:4d}  {r[0]} {r[1]} passed {r[2]} failed {r[3]} ignored")
print(f"\nTARGETS with 'running N': {len(rows)}   total passed {P} failed {F} ignored {I}")
print("running 0 targets:", zero)
print("bench targets (harness=false, no 'running N'):", len(bench))
for b in bench: print("  ",b)
print("FAILED/panicked lines:", sum(1 for l in lines if 'FAILED' in l or 'panicked at' in l))
