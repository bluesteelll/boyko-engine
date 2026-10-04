"""v5: the p0 re-run stage - pending slots vs slots actually re-run, waits inside the stage, short cells caused."""
import collections
import vlib as L

o = L.Out()
recs = L.records()
procs, used, dropped, slots = L.select('C4-AB', recs)
p0r = [p for p in procs if p['pass_'] == 0 and p['attempt'] == 'rerun']
pend = [k for k, ps in slots.items() if k[0] == 0 and not [p for p in ps if p['attempt'] == 'original'][0]['clean']]
o(f'p0: slots with an unclean original {len(pend)}; slots re-run {len({(p["pass_"], p["seq"]) for p in p0r})}; '
  f're-run processes {len(p0r)}; clean re-runs {sum(1 for p in p0r if p["clean"])}')
o(f'p0 waited_s inside the re-run stage: {sum(p["rec"].get("waited_s", 0) for p in p0r):.1f} s; '
  f'first start {min(p["rec"]["start"] for p in p0r)} last end {max(p["rec"]["end"] for p in p0r)}')
never = [k for k in pend if not any(p['attempt'] == 'rerun' for p in slots[k])]
o(f'p0 slots never re-run: {len(never)} (all dropped: {all(k in [d[0] for d in dropped] for k in never)})')
o(f'p0 dropped slots: {len(dropped)} = never re-run {len(never)} + re-run but unclean again {len(dropped) - len(never)}')
o.save('v5_rerun_stage.txt')
