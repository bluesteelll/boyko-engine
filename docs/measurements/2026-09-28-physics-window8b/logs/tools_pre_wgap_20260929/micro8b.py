"""Window 8b (2026-09-29): the micro-row validity rules in ONE place, used by the driver (per process), the gate
(one sample per row), counts8b.py (per cell, after the window) and selftest8b.py (red controls).

The rules for omega_b v2 are s7-omega2/cut.md "Window 8b: the S7 commands as amended": a row is void if
work_ns_calibrated is outside [500, 1000] ns, exactly_once is not true, lost_wakeups != 0, or a park row at gap_us > 0
whose parks_per_region_median is 0 on every rep; a park row at gap 0 with no park is REPORTED ("no park taken"), not
voided. "Every rep" is read as every process of the cell (the window's K), because parks_per_region_median is already a
per-process statistic (the median over that process's regions): one process at 0 is a receipt, not a void.

Every rule applies only when the row names it (rows8b.extra.json: expect_summaries / expect_fields / expect_bench /
expect_route / expect_stages, as window 8; void_rules: calibration, ranges, park_gap), so window 8's v1 rows keep
exactly their old checks. Reads no timing field: work_ns_calibrated is the calibration receipt (the work each block
was asked to do), parks_per_region_median is a count."""
import json


def parse(text):
    """(SUMMARY dicts, CALIBRATION dicts) of one micro process's decoded stdout; an unparsable line is kept, marked."""
    ss, cal = [], []
    for line in text.splitlines():
        for tag, out in (('SUMMARY ', ss), ('CALIBRATION ', cal)):
            if line.startswith(tag):
                try:
                    out.append(json.loads(line[len(tag):]))
                except ValueError:
                    out.append({'unparsed': line})
    return ss, cal


def park_key(s):
    return (f'P{s.get("participants")}/stages{s.get("stages")}/bpp{s.get("blocks_per_participant")}/'
            f'gap{s.get("gap_us")}/{s.get("route")}')


def process_rules(row, ss, cal, test=False):
    """(why, notes) for one process. `why`: the invalid reasons - window 8's expect_* checks verbatim, then the
    void_rules the row names. `notes`: the park receipt per park row; never a reason by itself (the gap rule is per
    cell: cell_park_rule)."""
    why = []
    if row.get('expect_summaries') is not None and len(ss) != row['expect_summaries'] and not test:
        why.append(f'{len(ss)} SUMMARY lines, expected {row["expect_summaries"]}')
    if not ss:
        why.append('no SUMMARY line')
    exp = dict(row.get('expect_fields') or {})
    for k in ('bench', 'route', 'stages'):
        if row.get(f'expect_{k}') is not None:
            exp[k] = row[f'expect_{k}']
    bad = [x for x in ss if any(x.get(k) != v for k, v in exp.items())]
    if bad:
        fields = sorted({k for x in bad for k, v in exp.items() if x.get(k) != v})
        why.append(f'{len(bad)} SUMMARY line(s) not matching {exp} (fields {fields})')
    vr = row.get('void_rules') or {}
    notes = {}
    if vr.get('calibration') is not None:
        want = vr['calibration']
        if len(cal) != 1:
            why.append(f'{len(cal)} CALIBRATION lines, expected 1')
        elif any(cal[0].get(k) != v for k, v in want.items()):
            why.append(f'CALIBRATION fields {sorted(k for k, v in want.items() if cal[0].get(k) != v)} not matching {want}')
    for field, (lo, hi) in (vr.get('ranges') or {}).items():
        for src, lines in (('CALIBRATION', cal), ('SUMMARY', ss)):
            out = [x.get(field) for x in lines
                   if isinstance(x.get(field), bool) or not isinstance(x.get(field), (int, float)) or not lo <= x[field] <= hi]
            if out:
                why.append(f'{src} {field} {out[:3]} outside [{lo}, {hi}] ({len(out)} line(s))')
    if vr.get('park_gap'):
        park = [{'key': park_key(x), 'gap_us': x.get('gap_us'), 'parks': x.get('parks_per_region_median')}
                for x in ss if x.get('helper') == 'park']
        notes['park'] = park
        notes['no_park_taken'] = [p['key'] for p in park if p['gap_us'] == 0 and p['parks'] == 0]
        notes['no_park_at_gap'] = [p['key'] for p in park if (p['gap_us'] or 0) > 0 and p['parks'] == 0]
        miss = [p['key'] for p in park if not isinstance(p['parks'], int) or isinstance(p['parks'], bool)]
        if miss:
            why.append(f'park rows without an integer parks_per_region_median: {miss[:3]}')
        if not park and not test:
            why.append('park_gap rule named but no park row (helper "park") in the output')
    return why, notes


def cell_park_rule(recs):
    """Over every rep (process record carrying micro_notes) of each (row, park config): (void, no_park_taken, seen).
    void: gap_us > 0 and parks_per_region_median == 0 on EVERY rep -> the cell is void. no_park_taken: gap_us == 0 and
    0 on every rep -> reported, not void. seen: {(row, key): [parks per rep]} for the record."""
    seen = {}
    gap = {}
    for r in recs:
        for p in ((r.get('micro_notes') or {}).get('park') or []):
            seen.setdefault((r['row'], p['key']), []).append(p['parks'])
            gap[(r['row'], p['key'])] = p['gap_us']
    void, nopark = [], []
    for k, ns in sorted(seen.items()):
        if ns and all(n == 0 for n in ns):
            (void if (gap[k] or 0) > 0 else nopark).append((k[0], k[1], len(ns)))
    return void, nopark, seen
