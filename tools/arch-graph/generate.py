#!/usr/bin/env python3
"""Architecture graph generator for boyko-engine.

Derives the project graph from CODE, not from docs:

  * crate dependency edges      <- Cargo.toml `[dependencies]` (normal vs dev)
  * module trees                <- `mod x;` declarations resolved to files
  * file-to-file import edges   <- every `use` statement, resolved through the
                                   module maps of THIS crate (`crate::`,
                                   `self::`, `super::`) or the target crate
                                   (`boyko_ecs::ecs::core::schedule::...`)
  * file size / LOC             <- line counts (refactor candidates flagged)
  * one-line summaries          <- the file's own leading `//!` doc comment

The output (`arch-graph.json`) is consumed by `index.html` (interactive
viewer) and is designed to be read directly by LLM agents: every node carries
a stable id, a kind, a path, and precomputed fan-in/fan-out.

Usage:
    python generate.py                 # scan src/ of every workspace crate
    python generate.py --dump-tree     # print module trees (for authoring
                                       # systems.json)
    python generate.py --include-tests # also scan tests/, benches/, examples/

Method notes / deliberate approximations (stated so no reader over-trusts):
  * `use` paths are resolved to the DEEPEST module that prefixes them; a path
    that matches no module (a re-export, a prelude glob) lands on the crate
    root (lib.rs). Re-export edges therefore all point at lib.rs / prelude.
  * inline `mod x { .. }` blocks are treated as part of the declaring file.
  * macros that emit `use` tokens are invisible to this scan.
  * scope is `src/` unless --include-tests is passed.

Stdlib only; Python 3.11+ (tomllib).
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

SCHEMA = "boyko-arch-graph/1"

# LOC thresholds for the refactor-candidate flags.
XL_LINES = 5000
LARGE_LINES = 2500
BIG_LINES = 1000

# external crates we keep as nodes (aggregated); everything else external is
# still counted, just kept in `externals` without a graph node.
EXTERNAL_NODE_CAP = 80


# --------------------------------------------------------------------------
# source tokenizer: mask comments and string bodies so `use`/`mod` parsing
# cannot be poisoned by prose or string contents. Newlines are preserved.
# --------------------------------------------------------------------------

def mask_source(text: str) -> str:
    out = list(text)
    i, n = 0, len(text)
    STATE_CODE, STATE_LINE, STATE_BLOCK, STATE_STR, STATE_RAW = range(5)
    state = STATE_CODE
    raw_hashes = 0
    while i < n:
        c = text[i]
        if state == STATE_CODE:
            if c == '/' and i + 1 < n and text[i + 1] == '/':
                out[i] = out[i + 1] = ' '
                state = STATE_LINE
                i += 2
                continue
            if c == '/' and i + 1 < n and text[i + 1] == '*':
                out[i] = out[i + 1] = ' '
                state = STATE_BLOCK
                i += 2
                continue
            if c == 'r' and i + 1 < n and text[i + 1] in '"#':
                # possible raw string r"..." / r#"..."# / br#"..."#
                j = i
                if j > 0 and text[j - 1] in 'bB':
                    j -= 1
                if text[j] in 'bB':
                    j -= 0
                # count hashes between r and quote
                k = i + 1
                hashes = 0
                while k < n and text[k] == '#':
                    hashes += 1
                    k += 1
                if k < n and text[k] == '"' and text[i] == 'r':
                    # mask 'r' + hashes + quote
                    for m in range(i, k + 1):
                        out[m] = ' '
                    raw_hashes = hashes
                    state = STATE_RAW
                    i = k + 1
                    continue
            if c == '"':
                out[i] = ' '
                state = STATE_STR
                i += 1
                continue
            if c == "'":
                # char literal vs lifetime: 'x' / '\n' / '\u{1}' are chars;
                # 'a (identifier after) is a lifetime.
                if i + 1 < n:
                    nxt = text[i + 1]
                    if nxt == '\\':
                        # '\n' etc: skip to closing quote
                        j = i + 2
                        while j < n and text[j] not in "'\n":
                            j += 1
                        if j < n and text[j] == "'":
                            for m in range(i, j + 1):
                                out[m] = ' '
                            i = j + 1
                            continue
                    elif nxt != "'" and (nxt.isalnum() or nxt == '_'):
                        if i + 2 < n and text[i + 2] == "'":
                            out[i] = out[i + 1] = out[i + 2] = ' '
                            i += 3
                            continue
                        # lifetime like 'a — leave as-is
            i += 1
            continue
        if state == STATE_LINE:
            if c == '\n':
                state = STATE_CODE
            else:
                out[i] = ' '
            i += 1
            continue
        if state == STATE_BLOCK:
            if c == '*' and i + 1 < n and text[i + 1] == '/':
                out[i] = out[i + 1] = ' '
                state = STATE_CODE
                i += 2
                continue
            if c != '\n':
                out[i] = ' '
            i += 1
            continue
        if state == STATE_STR:
            if c == '\\':
                out[i] = ' '
                if i + 1 < n and text[i + 1] != '\n':
                    out[i + 1] = ' '
                i += 2
                continue
            if c == '"':
                out[i] = ' '
                state = STATE_CODE
            elif c != '\n':
                out[i] = ' '
            i += 1
            continue
        if state == STATE_RAW:
            # inside r##"..."## : look for '"' + raw_hashes '#'
            if c == '"':
                ok = all(text[i + 1 + h] == '#' for h in range(raw_hashes))
                if ok:
                    for m in range(i, i + 1 + raw_hashes):
                        out[m] = ' '
                    state = STATE_CODE
                    i += 1 + raw_hashes
                    continue
            if c != '\n':
                out[i] = ' '
            i += 1
            continue
    return ''.join(out)


# --------------------------------------------------------------------------
# per-file extraction
# --------------------------------------------------------------------------

MOD_DECL_RE = re.compile(
    r'(?:^|\n)[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;',
)
PATH_ATTR_RE = re.compile(
    r'#\[\s*path\s*=\s*"([^"]+)"\s*\][ \t]*(?:\n[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?)?'
    r'mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;'
)
USE_KEYWORD_RE = re.compile(r'\buse\b')


def split_top_level(s: str, sep: str) -> list[str]:
    parts, depth, buf = [], 0, []
    for ch in s:
        if ch == '{':
            depth += 1
        elif ch == '}':
            depth -= 1
        if ch == sep and depth == 0:
            parts.append(''.join(buf))
            buf = []
        else:
            buf.append(ch)
    parts.append(''.join(buf))
    return parts


def expand_use_tree(seg: str, cap: int = 128) -> list[str]:
    """`a::{b, c::{d, e}}` -> all concrete paths."""
    seg = seg.strip()
    brace = seg.find('{')
    if brace == -1:
        return [seg] if seg else []
    prefix = seg[:brace].strip().rstrip(':')
    # find matching brace
    depth = 0
    for idx in range(brace, len(seg)):
        if seg[idx] == '{':
            depth += 1
        elif seg[idx] == '}':
            depth -= 1
            if depth == 0:
                inner = seg[brace + 1:idx]
                tail = seg[idx + 1:].strip()
                out = []
                for item in split_top_level(inner, ','):
                    item = item.strip()
                    if not item:
                        continue
                    # strip ` as alias`
                    if ' as ' in item:
                        item = item.split(' as ')[0].strip()
                    joined = f'{prefix}::{item}' if prefix else item
                    out.extend(expand_use_tree(joined, cap - len(out)))
                    if len(out) >= cap:
                        break
                if tail:
                    # e.g. `a::{b}::c` — rare; glue the tail
                    out = [f'{p}{tail}' for p in out]
                return out
    return []


def parse_use_paths(clean: str) -> list[str]:
    paths: list[str] = []
    for m in USE_KEYWORD_RE.finditer(clean):
        j = m.start()
        if j > 0 and (clean[j - 1].isalnum() or clean[j - 1] == '_'):
            continue  # part of an identifier
        k = m.end()
        depth = 0
        buf: list[str] = []
        end = None
        n = len(clean)
        while k < n:
            c = clean[k]
            if c == '{':
                depth += 1
                buf.append(c)
            elif c == '}':
                depth -= 1
                buf.append(c)
            elif depth == 0 and (c == ';' or c == '\n'):
                end = k
                break
            else:
                buf.append(c)
            k += 1
        if end is None:
            break
        stmt = ''.join(buf).strip()
        # multi-line `use a::{\n b,\n c\n};` groups: flatten whitespace before
        # the shape check, then let expand_use_tree handle the braces
        stmt_flat = re.sub(r'\s+', ' ', stmt)
        if stmt_flat and re.match(r'^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z0-9_:{}*, ]+)*$', stmt_flat.replace(' as ', ' ')):
            paths.extend(expand_use_tree(stmt_flat))
        i = end
    return paths


def first_doc_summary(raw: str) -> str:
    lines = raw.split('\n')
    out = []
    if not lines or not lines[0].startswith('//!'):
        return ''
    for ln in lines:
        if ln.startswith('//!'):
            t = ln[3:].strip()
            if t:
                out.append(t)
            # stop at an empty `//!` separator or after the first paragraph
            # heading — keep it to one line of real content
            if len(out) >= 1 and (not t or t.startswith('#')):
                break
        else:
            break
    s = ' '.join(out).strip()
    if len(s) > 280:
        s = s[:277] + '...'
    return s


def first_doc_paragraph(raw: str) -> str:
    """The file's own leading `//!` doc block, up to the first non-doc line —
    the code-authored description of what this file is."""
    lines = raw.split('\n')
    started = False
    out: list[str] = []
    for ln in lines:
        s = ln.strip()
        if not started:
            if s == '' or s.startswith('#[') or s.startswith('#!['):
                continue  # attributes / blanks before the doc block
            if s.startswith('//!'):
                started = True
            else:
                break  # code began with no module doc
        if not s.startswith('//!'):
            break
        t = s[3:].strip()
        if not t:
            if out:
                break  # blank doc line ends the first paragraph
            continue
        if t.startswith('#'):
            break  # a markdown heading ends the intro paragraph
        out.append(t)
        if sum(len(x) for x in out) > 600:
            break
    text = ' '.join(out).strip()
    text = re.sub(r'\s+', ' ', text)
    if len(text) > 620:
        text = text[:617] + '...'
    return text


PUB_ITEM_RE = re.compile(
    r'(?:^|\n)[ \t]*pub(?:\([^)]*\))?[ \t]+'
    r'(?:const|static|struct|enum|union|trait|fn|type|mod)[ \t]+'
    r'([A-Za-z_][A-Za-z0-9_]*)'
)


def extract_pub_items(clean: str, cap: int = 14) -> list[str]:
    """Top-level `pub` item names in declaration order (from masked source)."""
    seen: list[str] = []
    for m in PUB_ITEM_RE.finditer(clean):
        name = m.group(1)
        if name not in seen:
            seen.append(name)
            if len(seen) >= cap:
                break
    return seen


class SourceFile:
    def __init__(self, path: Path, crate: str, unit: str = ''):
        self.path = path
        self.crate = crate
        self.unit = unit                # owning unit key (set at scan time)
        raw = path.read_text(encoding='utf-8', errors='replace')
        self.loc = raw.count('\n') + (0 if raw.endswith('\n') or not raw else 1)
        self.summary = first_doc_summary(raw)
        self.doc = first_doc_paragraph(raw)
        clean = mask_source(raw)
        self.pub_items = extract_pub_items(clean)
        self.mods = [m.group(1) for m in MOD_DECL_RE.finditer(clean)]
        # inline `mod x {` blocks are NOT file children — drop them
        inline = set(re.findall(r'\bmod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*\{', clean))
        self.mods = [m for m in self.mods if m not in inline]
        self.path_attrs = {name: rel for rel, name in PATH_ATTR_RE.findall(raw)}
        self.uses = parse_use_paths(clean)
        self.module: tuple[str, ...] = ()   # filled by the tree walk


# --------------------------------------------------------------------------
# workspace scan
# --------------------------------------------------------------------------

class Crate:
    """One compilation unit. A crate directory with lib.rs + main.rs + bins
    yields several units (the lib unit owns the crate name in `use` paths)."""

    def __init__(self, key: str, lib_name: str, root_dir: Path, roots: list[Path], is_bin: bool):
        self.key = key                  # stable unit key (dir name or dir__main / dir__bin_x)
        self.lib_name = lib_name        # name code writes in `use` (underscores)
        self.root_dir = root_dir
        self.roots = roots              # the unit's root file(s)
        self.is_bin = is_bin
        self.files: list[SourceFile] = []
        self.module_map: dict[tuple[str, ...], Path] = {}
        self.cargo_deps: list[tuple[str, bool]] = []   # (lib_name, is_dev)


def scan_workspace(root: Path, include_tests: bool):
    units: dict[str, Crate] = {}
    libname_to_key: dict[str, str] = {}

    def load_crate_units(dirpath: Path):
        toml_path = dirpath / 'Cargo.toml'
        if not toml_path.exists():
            return
        data = tomllib.loads(toml_path.read_text(encoding='utf-8'))
        pkg = data.get('package', {})
        lib_name = (pkg.get('name') or dirpath.name).replace('-', '_')

        src = dirpath / 'src'
        lib = src / 'lib.rs'
        main = src / 'main.rs'
        bindir = src / 'bin'
        bins = sorted(bindir.glob('*.rs')) if bindir.is_dir() else []

        unit_defs: list[tuple[str, list[Path], bool]] = []
        if lib.exists():
            unit_defs.append((dirpath.name, [lib], False))
        if main.exists():
            unit_defs.append((f'{dirpath.name}__main', [main], True))
        for b in bins:
            unit_defs.append((f'{dirpath.name}__bin_{b.stem}', [b], True))
        if not unit_defs:
            return

        deps: list[tuple[str, bool]] = []
        for dep, spec in data.get('dependencies', {}).items():
            if isinstance(spec, dict) and spec.get('path'):
                deps.append((dep.replace('-', '_'), False))
        for dep, spec in data.get('dev-dependencies', {}).items():
            if isinstance(spec, dict) and spec.get('path'):
                deps.append((dep.replace('-', '_'), True))

        for key, roots, is_bin in unit_defs:
            crate = Crate(key, lib_name, dirpath, roots, is_bin)
            if not is_bin:
                crate.cargo_deps = deps
                libname_to_key.setdefault(lib_name, key)
            elif lib_name not in libname_to_key:
                libname_to_key[lib_name] = key   # bins-only crate
            # file scope per unit
            if not is_bin:
                scope = [p for p in sorted(src.rglob('*.rs'))
                         if p != main and bindir not in p.parents]
            else:
                scope = roots
            for p in scope:
                sf = SourceFile(p, lib_name, key)
                crate.files.append(sf)
            units[key] = crate

    for toml in sorted(root.glob('crates/*/Cargo.toml')):
        load_crate_units(toml.parent)
    # thin root binary
    root_main = root / 'src' / 'main.rs'
    if root_main.exists():
        c = Crate('__rootbin__', 'boyko_engine_bin', root, [root_main], True)
        c.files.append(SourceFile(root_main, 'boyko_engine_bin', c.key))
        units[c.key] = c
        libname_to_key.setdefault('boyko_engine_bin', c.key)

    # build module maps
    for crate in units.values():
        by_path = {f.path: f for f in crate.files}
        seen: set[Path] = set()
        queue: list[tuple[Path, tuple[str, ...]]] = []
        for r in crate.roots:
            if r in by_path:
                queue.append((r, ()))
        while queue:
            fpath, modpath = queue.pop(0)
            if fpath in seen or fpath not in by_path:
                continue
            seen.add(fpath)
            sf = by_path[fpath]
            sf.module = modpath
            crate.module_map[modpath] = fpath
            for name in sf.mods:
                rel = sf.path_attrs.get(name)
                child = resolve_mod_file(fpath, name, rel)
                if child is not None and child in by_path:
                    queue.append((child, modpath + (name,)))
        # files never reached by the mod walk (bin roots, orphaned test mods
        # pulled in via #[path]) keep module = their path relative to src
        for sf in crate.files:
            if sf.path not in seen:
                rel = sf.path.relative_to(crate.root_dir / 'src').with_suffix('')
                parts = tuple(p for p in rel.parts if p != 'mod')
                sf.module = parts

    if include_tests:
        for crate in units.values():
            if crate.is_bin:
                continue
            for sub in ('tests', 'benches', 'examples'):
                base = crate.root_dir / sub
                if not base.is_dir():
                    continue
                for p in sorted(base.rglob('*.rs')):
                    sf = SourceFile(p, crate.lib_name)
                    rel = p.relative_to(base).with_suffix('')
                    sf.module = (sub,) + tuple(rel.parts)
                    crate.files.append(sf)
    return units, libname_to_key


def resolve_mod_file(containing: Path, name: str, path_attr: str | None) -> Path | None:
    d = containing.parent
    if path_attr is not None:
        cand = d / path_attr
        return cand if cand.exists() else None
    if containing.name in ('mod.rs', 'lib.rs', 'main.rs'):
        cands = [d / f'{name}.rs', d / name / 'mod.rs']
    else:
        sub = d / containing.stem
        cands = [sub / f'{name}.rs', sub / name / 'mod.rs']
    for c in cands:
        if c.exists():
            return c
    return None


# --------------------------------------------------------------------------
# use resolution
# --------------------------------------------------------------------------

def resolve_in_crate(crate: Crate, segs: list[str]):
    """Deepest module-prefix match; falls back to the crate root file."""
    best = ()
    for i in range(len(segs), 0, -1):
        key = tuple(segs[:i])
        if key in crate.module_map:
            best = key
            break
    return crate.module_map.get(best, crate.roots[0] if crate.roots else None)


def resolve_use(path: str, current: SourceFile, units, libname_to_key):
    """Returns ('file', Path) | ('external', name) | None."""
    segs = path.split('::')
    segs = [s.strip() for s in segs if s.strip()]
    if not segs:
        return None
    head = segs[0]
    if head in ('std', 'core', 'alloc'):
        return ('external', head)
    if head == 'crate':
        target = resolve_in_crate(units[current.unit], segs[1:])
        return ('file', target) if target else None
    if head in ('self', 'super'):
        base = list(current.module)
        rest = segs
        while rest and rest[0] in ('self', 'super'):
            if rest[0] == 'super':
                if not base:
                    return None
                base.pop()
            rest = rest[1:]
        target = resolve_in_crate(units[current.unit], base + rest)
        return ('file', target) if target else None
    if head in libname_to_key:
        target_crate = units[libname_to_key[head]]
        target = resolve_in_crate(target_crate, segs[1:])
        return ('file', target) if target else None
    if len(segs) > 1 or re.match(r'^[a-z_][a-z0-9_]*$', head):
        return ('external', head)
    return None


# --------------------------------------------------------------------------
# systems assignment (systems.json authored from --dump-tree output)
# --------------------------------------------------------------------------

def load_systems(path: Path):
    if not path.exists():
        return []
    data = json.loads(path.read_text(encoding='utf-8'))
    return data.get('systems', [])


def system_for(crate: Crate, module: tuple[str, ...], systems) -> str | None:
    mod_str = '::'.join(module)
    best = None
    best_len = -1
    for s in systems:
        for a in s.get('assign', []):
            if a.get('crate') != crate.lib_name:
                continue
            pref = a.get('module', '')
            if pref == '' or mod_str == pref or mod_str.startswith(pref + '::'):
                if len(pref) > best_len:
                    best = s['id']
                    best_len = len(pref)
    return best


# --------------------------------------------------------------------------
# graph assembly
# --------------------------------------------------------------------------

def build_graph(root: Path, include_tests: bool, systems: list[dict]):
    units, libname_to_key = scan_workspace(root, include_tests)
    syslabel = {s['id']: s.get('label', s['id']) for s in systems}
    syslayer = {s['id']: s.get('layer', '') for s in systems}

    # ---- resolve all uses into raw edge list -----------------------------
    # edge key: (src_path_str, dst_path_str) -> weight
    edges: dict[tuple[str, str], int] = defaultdict(int)
    edges_uses: dict[tuple[str, str], list[str]] = defaultdict(list)
    external_weight: dict[str, int] = defaultdict(int)
    unresolved = 0
    for crate in units.values():
        for sf in crate.files:
            for u in sf.uses:
                r = resolve_use(u, sf, units, libname_to_key)
                if r is None:
                    unresolved += 1
                    continue
                kind, val = r
                if kind == 'external':
                    external_weight[val] += 1
                else:
                    if val != sf.path:  # ignore self-imports
                        key = (str(sf.path), str(val))
                        edges[key] += 1
                        if u not in edges_uses[key] and len(edges_uses[key]) < 6:
                            edges_uses[key].append(u)

    # ---- file nodes -------------------------------------------------------
    file_nodes = {}
    for crate in units.values():
        for sf in crate.files:
            rel = str(sf.path.relative_to(root)).replace('\\', '/')
            sys_id = system_for(crate, sf.module, systems)
            size = 'xl' if sf.loc >= XL_LINES else 'l' if sf.loc >= LARGE_LINES \
                else 'm' if sf.loc >= BIG_LINES else 's'
            # description: the file's own doc block; a synthesized fallback so
            # EVERY node carries text
            desc = sf.doc
            if not desc:
                parts = []
                if sys_id:
                    parts.append(f"Part of the '{syslabel.get(sys_id, sys_id)}' system.")
                parts.append(f'{sf.loc} lines of '
                             f'{crate.lib_name}.')
                if sf.pub_items:
                    parts.append('Defines: ' + ', '.join(sf.pub_items[:8]) + '.')
                desc = ' '.join(parts)
            file_nodes[str(sf.path)] = {
                'id': 'file:' + rel,
                'kind': 'file',
                'path': rel,
                'crate': crate.lib_name,
                'module': '::'.join(sf.module),
                'system': sys_id,
                'loc': sf.loc,
                'size_class': size,
                'refactor_candidate': sf.loc >= LARGE_LINES,
                'summary': sf.summary,
                'description': desc,
                'symbols': sf.pub_items,
            }

    # ---- crate nodes (aggregate units by lib name) ------------------------
    crate_nodes = {}
    lib_units = {u.lib_name: u for u in units.values() if not u.is_bin}
    for lib_name, crate in lib_units.items():
        members = [u for u in units.values() if u.lib_name == lib_name]
        total = sum(f.loc for u in members for f in u.files)
        nfiles = sum(len(u.files) for u in members)
        sysids = sorted({fn['system'] for fn in file_nodes.values()
                         if fn['crate'] == lib_name and fn['system']})
        # the lib unit's root file IS lib.rs — its doc block is the crate doc
        root_sf = next((f for f in crate.files if f.path == crate.roots[0]), None)
        desc = (root_sf.doc if root_sf else '') or \
            f'{lib_name}: {nfiles} files, {total} lines.'
        crate_nodes[lib_name] = {
            'id': 'crate:' + lib_name,
            'kind': 'crate',
            'name': lib_name,
            'dir': f'crates/{crate.key}',
            'loc': total,
            'files': nfiles,
            'systems': sysids,
            'description': desc,
            'cargo_deps': [
                {'to': d, 'dev': dev} for d, dev in crate.cargo_deps
            ],
        }

    # ---- system nodes -----------------------------------------------------
    system_nodes = {}
    for s in systems:
        sid = s['id']
        members = [fn for fn in file_nodes.values() if fn['system'] == sid]
        top = sorted(members, key=lambda m: -m['loc'])[:6]
        desc = s.get('summary', '')
        if top:
            desc += ' Key files (by size): ' + ', '.join(
                f"{m['path']} ({m['loc']})" for m in top) + '.'
        system_nodes[sid] = {
            'id': 'system:' + sid,
            'kind': 'system',
            'label': s.get('label', sid),
            'summary': s.get('summary', ''),
            'description': desc,
            'crates': sorted({m['crate'] for m in members}),
            'files': len(members),
            'loc': sum(m['loc'] for m in members),
            'layer': s.get('layer', ''),
            'color': s.get('color', '#97c2fc'),
        }

    # ---- fan in/out -------------------------------------------------------
    fan_out = defaultdict(int)
    fan_in = defaultdict(int)
    for (src, dst), w in edges.items():
        fan_out[src] += 1
        fan_in[dst] += 1
    for p, n in file_nodes.items():
        n['fan_out'] = fan_out.get(p, 0)
        n['fan_in'] = fan_in.get(p, 0)

    # ---- aggregated edges (crate & system level) --------------------------
    def sys_of_path(p: str):
        return file_nodes.get(p, {}).get('system')

    def crate_of_path(p: str):
        return file_nodes.get(p, {}).get('crate')

    crate_edges: dict[tuple[str, str, str], int] = defaultdict(int)
    system_edges: dict[tuple[str, str, str], int] = defaultdict(int)
    crate_examples: dict[tuple[str, str], list] = defaultdict(list)
    system_examples: dict[tuple[str, str], list] = defaultdict(list)
    for (src, dst), w in edges.items():
        cs, cd = crate_of_path(src), crate_of_path(dst)
        ss, sd = sys_of_path(src), sys_of_path(dst)
        pair = (file_nodes[src]['path'], file_nodes[dst]['path'], w)
        if cs and cd and cs != cd:
            crate_edges[(cs, cd, 'imports')] += w
            crate_examples[(cs, cd)].append(pair)
        if ss and sd and ss != sd:
            system_edges[(ss, sd, 'imports')] += w
            system_examples[(ss, sd)].append(pair)
    for lib_name, crate in lib_units.items():
        for dep, dev in crate.cargo_deps:
            if dep in lib_units:
                crate_edges[(lib_name, dep, 'cargo' + ('-dev' if dev else ''))] += 1

    def edge_list(d, examples, name_of):
        out = []
        for (a, b, k), w in sorted(d.items(), key=lambda kv: -kv[1]):
            e = {'from': name_of(a), 'to': name_of(b),
                 'kind': k, 'weight': w if w > 0 else 1}
            if k == 'imports':
                exs = sorted(examples.get((a, b), []), key=lambda p: -p[2])[:5]
                npairs = len(examples.get((a, b), []))
                e['description'] = (
                    f'{a} imports from {b} in {npairs} distinct file pairs '
                    f'({w} import sites). Heaviest: '
                    f'{exs[0][0]} -> {exs[0][1]}' if exs else
                    f'{a} imports from {b}.')
                if exs:
                    e['examples'] = [{'from': x[0], 'to': x[1], 'w': x[2]}
                                     for x in exs]
            elif k == 'cargo':
                e['description'] = (f'{a} declares {b} in Cargo.toml '
                                    f'[dependencies] (compile-time edge).')
            else:
                e['description'] = (f'{a} declares {b} in Cargo.toml '
                                    f'[dev-dependencies] (tests/benches only).')
            out.append(e)
        return out

    graph = {
        'schema': SCHEMA,
        'generated': datetime.datetime.now().isoformat(timespec='seconds'),
        'root': str(root),
        'scope': 'src + tests + benches + examples' if include_tests else 'src',
        'method': {
            'edges': 'every `use` statement resolved to the deepest matching '
                     'module; re-exports land on the crate root (lib.rs)',
            'systems': 'assigned by longest module-prefix match from '
                       'systems.json (authored from the module trees, keyed '
                       'by real crate+module paths)',
            'loc': 'line count of the file as-is',
            'descriptions': 'file.description / crate.description are the '
                            'code-authored leading `//!` doc block of that '
                            'file (verbatim, no doc/ markdown consulted); '
                            'files without one get a synthesized sentence '
                            'from their pub symbols; file.symbols lists '
                            'top-level `pub` item names; edge.uses holds the '
                            'raw `use` path strings behind the edge',
        },
        'counts': {
            'files': len(file_nodes),
            'crates': len(crate_nodes),
            'systems': len(system_nodes),
            'file_edges': len(edges),
            'unresolved_uses': unresolved,
        },
        'systems': list(system_nodes.values()),
        'crates': list(crate_nodes.values()),
        'files': list(file_nodes.values()),
        'edges': [
            {'from': 'file:' + file_nodes[s]['path'] if s in file_nodes else 'file:' + s,
             'to': 'file:' + file_nodes[d]['path'] if d in file_nodes else 'file:' + d,
             'kind': 'imports', 'weight': w,
             'uses': edges_uses.get((s, d), [])}
            for (s, d), w in sorted(edges.items())
        ],
        'crate_edges': edge_list(crate_edges, crate_examples, lambda n: 'crate:' + n),
        'system_edges': edge_list(system_edges, system_examples, lambda n: 'system:' + n),
        'externals': [
            {'name': name, 'weight': w}
            for name, w in sorted(external_weight.items(), key=lambda kv: -kv[1])
        ][:EXTERNAL_NODE_CAP],
    }
    return graph


# --------------------------------------------------------------------------
# dump mode: module trees for authoring systems.json
# --------------------------------------------------------------------------

def dump_trees(root: Path):
    units, _ = scan_workspace(root, False)
    for key, crate in sorted(units.items()):
        print(f'== {crate.lib_name}  ({len(crate.files)} files, '
              f'{sum(f.loc for f in crate.files)} loc)')
        for mod, f in sorted(crate.module_map.items(), key=lambda kv: kv[0]):
            rel = str(f.relative_to(crate.root_dir)).replace('\\', '/')
            loc = next(x.loc for x in crate.files if x.path == f)
            print(f'  {"::".join(mod) or "(root)":60s} {loc:6d}  {rel}')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--root', default=str(Path(__file__).resolve().parent.parent.parent))
    ap.add_argument('--out', default=str(Path(__file__).resolve().parent / 'arch-graph.json'))
    ap.add_argument('--dump-tree', action='store_true')
    ap.add_argument('--include-tests', action='store_true')
    args = ap.parse_args()
    root = Path(args.root).resolve()

    if args.dump_tree:
        dump_trees(root)
        return

    systems = load_systems(Path(__file__).resolve().parent / 'systems.json')
    graph = build_graph(root, args.include_tests, systems)
    Path(args.out).write_text(json.dumps(graph, indent=1), encoding='utf-8')
    # the same payload as a JS global so index.html also works from file://
    js_out = Path(args.out).with_suffix('.js')
    js_out.write_text('window.ARCH_GRAPH = ' +
                      json.dumps(graph, separators=(',', ':')) + ';\n',
                      encoding='utf-8')
    c = graph['counts']
    print(f'wrote {args.out} + {js_out.name}: {c["files"]} files, {c["crates"]} crates, '
          f'{c["systems"]} systems, {c["file_edges"]} import edges, '
          f'{c["unresolved_uses"]} unresolved uses')


if __name__ == '__main__':
    main()
