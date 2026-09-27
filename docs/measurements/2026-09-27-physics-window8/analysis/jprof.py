"""Own parser of Jolt profile_chart dumps: per-thread intervals per scope."""
import json
import re


def _js_to_json(txt):
    txt = re.sub(r'(?m)^(\s*)(\w+):', r'\1"\2":', txt)
    txt = re.sub(r',\s*([}\]])', r'\1', txt)
    return json.loads(txt)


def _grab(s, head, open_ch, close_ch):
    i = s.find(head)
    j = s.find(open_ch, i)
    depth, k, in_str = 0, j, False
    while k < len(s):
        ch = s[k]
        if in_str:
            if ch == chr(92):
                k += 2
                continue
            if ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                return s[j:k + 1]
        k += 1
    return None


def parse(path):
    s = open(path, encoding='utf-8', errors='replace').read()
    cps = float(re.search(r'var\s+cycles_per_second\s*=\s*([\d.]+)', s).group(1))
    threads = _js_to_json(_grab(s, 'var threads', '[', ']'))
    agg = _js_to_json(_grab(s, 'var aggregated', '{', '}'))
    names = [n.replace('&lt;', '<').replace('&gt;', '>') for n in agg['name']]
    ivs = []  # (thread index, name, start_ms, end_ms, depth)
    t0 = min(min(t['start']) for t in threads if t['start'])
    for ti, t in enumerate(threads):
        for a, st, cy, d in zip(t['aggregator'], t['start'], t['cycles'], t['depth']):
            ivs.append((ti, names[a], (st - t0) * 1e3 / cps, (st + cy - t0) * 1e3 / cps, d))
    return {'cps': cps, 'threads': [t['thread_name'] for t in threads], 'ivs': ivs}


def union(iv):
    iv = sorted(iv)
    tot, cs, ce = 0.0, None, None
    for a, b in iv:
        if ce is None or a > ce:
            if ce is not None:
                tot += ce - cs
            cs, ce = a, b
        else:
            ce = max(ce, b)
    if ce is not None:
        tot += ce - cs
    return tot
