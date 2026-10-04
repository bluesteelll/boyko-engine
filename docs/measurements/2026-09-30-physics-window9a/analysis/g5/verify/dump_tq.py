import json, os, statistics
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'v_g5.py'), encoding='utf-8').read().split("\nP(\"\n=== B1")[0])
us = [p for p in used if p['row'] == 'C4-JD-armed' and p['W'] == 8]
json.dump({'mean': [statistics.fmean(p['c']['phys_bp_query_ns'][100:500]) / 1e6 for p in us],
           'median': [statistics.median(p['c']['phys_bp_query_ns'][100:500]) / 1e6 for p in us]},
          open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'v_g5_tq.json'), 'w'))
