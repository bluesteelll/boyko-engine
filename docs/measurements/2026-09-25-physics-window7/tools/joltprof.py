"""Parse one Jolt profile chart dump (profile_chart_<tag>.html, one frame) into per-scope numbers.
Returns {'cycles_per_second', 'threads': [...], 'scopes': {name: {'calls', 'cpu_ms', 'wall_ms', 'depth_min'}}}.
cpu_ms = the sum over every sample of the scope (all threads, children included), the profiler's own aggregate;
wall_ms = the extent of the union of the scope's intervals across threads (the time during which at least one
thread was inside that scope)."""
import json
import re
import sys


def _js_to_json(txt):
    txt = re.sub(r'(?m)^(\s*)(\w+):', r'\1"\2":', txt)
    txt = re.sub(r',\s*([}\]])', r'\1', txt)
    return json.loads(txt)


def _grab(s, head, open_ch, close_ch):
    i = s.find(head)
    if i < 0:
        return None
    j = s.find(open_ch, i)
    depth, k = 0, j
    in_str = False
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
    m = re.search(r'var\s+cycles_per_second\s*=\s*([\d.]+)', s)
    cps = float(m.group(1)) if m else None
    threads = _js_to_json(_grab(s, 'var threads', '[', ']'))
    agg = _js_to_json(_grab(s, 'var aggregated', '{', '}'))
    names = [n.replace('&lt;', '<').replace('&gt;', '>') for n in agg['name']]
    ivs = {}
    depth_min = {}
    for t in threads:
        for a, st, cy, d in zip(t['aggregator'], t['start'], t['cycles'], t['depth']):
            ivs.setdefault(a, []).append((st, st + cy))
            depth_min[a] = min(depth_min.get(a, 255), d)
    scopes = {}
    for a, nm in enumerate(names):
        iv = sorted(ivs.get(a, []))
        tot, cur_s, cur_e = 0, None, None
        for s0, e0 in iv:
            if cur_e is None or s0 > cur_e:
                if cur_e is not None:
                    tot += cur_e - cur_s
                cur_s, cur_e = s0, e0
            else:
                cur_e = max(cur_e, e0)
        if cur_e is not None:
            tot += cur_e - cur_s
        scopes[nm] = {'calls': agg['calls'][a], 'cpu_ms': 1e3 * agg['cycles_per_frame'][a] / cps if cps else None,
                      'wall_ms': 1e3 * tot / cps if cps else None, 'depth_min': depth_min.get(a)}
    return {'cycles_per_second': cps, 'threads': [t['thread_name'] for t in threads], 'scopes': scopes}


if __name__ == '__main__':
    r = parse(sys.argv[1])
    print('cps', r['cycles_per_second'], 'threads', r['threads'])
    for nm, v in sorted(r['scopes'].items(), key=lambda kv: -(kv[1]['cpu_ms'] or 0)):
        print(f"{v['cpu_ms']:9.4f} cpu  {v['wall_ms']:9.4f} wall  {v['calls']:6d} calls d{v['depth_min']}  {nm[:110]}")
