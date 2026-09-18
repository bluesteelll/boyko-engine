import json, glob, os, re, sys, collections
MQ = sys.argv[1]
for sha in sys.argv[2:]:
    log = open(f'{MQ}/logs/{sha}.verbose.stderr', encoding='utf-8', errors='replace').read()
    env = log.splitlines()[0]
    lines = [l for l in log.splitlines() if 'Running `' in l and 'rustc.exe' in l]
    with_cpu = [l for l in lines if l.rstrip('`').endswith('-C target-cpu=x86-64-v3') or '-C target-cpu=x86-64-v3' in l]
    other_cpu = [l for l in lines if re.search(r'target-cpu=(?!x86-64-v3)', l) or 'target-feature' in l]
    phys = [l for l in lines if '--crate-name boyko_physics ' in l]
    ph = phys[0] if phys else ''
    # fingerprint census over EVERY unit in the arm's target dir
    fps = glob.glob(f'D:/wt/_targets/mq-{sha}/release/.fingerprint/*/*.json')
    flags = collections.Counter()
    for f in fps:
        try: d = json.load(open(f))
        except Exception: continue
        if 'rustflags' in d: flags[tuple(d['rustflags'])] += 1
    print(f'{sha}: {env}')
    print(f'  verbose rustc lines={len(lines)} with target-cpu=x86-64-v3={len(with_cpu)} other cpu/feature flags={len(other_cpu)}')
    print(f'  boyko_physics rustc line: flags tail = ...{ph[ph.find("--test"):ph.find("--test")+7] if "--test" in ph else ""} ... {ph[-40:]}')
    print(f'  fingerprint units={sum(flags.values())} rustflags sets={dict(flags)}')
