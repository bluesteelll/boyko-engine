"""LSIF -> graphify edge converter (compiler-grade cross-file references).

Reads a rust-analyzer LSIF dump (newline-delimited JSON) and the existing
graphify graph (graphify-out/graph.json), resolves every cross-file
definition<-reference pair, maps both ends onto existing graphify AST nodes by
(file, nearest-enclosing line), and merges the resulting EXTRACTED edges back
into graph.json.

Why: the tree-sitter AST extractor sees one file at a time, so Rust method
calls (the majority of call sites) never produce cross-file edges; LLM
semantic agents recover some of that linkage at token cost. rust-analyzer's
name resolution is exact (methods, traits, generics) and free.

Usage:
    python tools/lsif_to_graphify.py [graphify-out/index.lsif]

Re-run after `rust-analyzer lsif . > graphify-out/index.lsif` whenever the
graph is rebuilt from scratch (the post-commit hook covers AST refreshes; a
full re-extract wipes these edges until this script is re-run).
"""

import json
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GRAPH = ROOT / 'graphify-out' / 'graph.json'
RELATION = 'references'


def load_lsif(path):
    """Returns (uri_by_doc, range_info, next_edges, def_result_of, items)."""
    uri_by_doc = {}
    range_line = {}            # range id -> start line (0-based)
    doc_of_range = {}          # range id -> document id (via contains edges)
    next_of = {}               # range id -> resultSet id
    defres_of = {}             # resultSet id -> definitionResult id
    items = defaultdict(list)  # definitionResult id -> [(doc id, range id)]

    # PowerShell 5.1 `>` redirection writes UTF-16 LE with BOM; a direct
    # rust-analyzer pipe gives UTF-8. Sniff the BOM.
    with open(path, 'rb') as probe:
        bom = probe.read(2)
    enc = 'utf-16' if bom == b'\xff\xfe' else 'utf-8'
    with open(path, encoding=enc) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            v = json.loads(line)
            typ = v.get('type')
            if typ == 'vertex':
                lbl = v.get('label')
                if lbl == 'document':
                    uri_by_doc[v['id']] = v.get('uri', '')
                elif lbl == 'range':
                    range_line[v['id']] = v['start']['line']
            elif typ == 'edge':
                lbl = v.get('label')
                if lbl == 'contains':
                    for r in v.get('inVs', []):
                        doc_of_range[r] = v['outV']
                elif lbl == 'next':
                    next_of[v['outV']] = v['inV']
                elif lbl == 'textDocument/definition':
                    defres_of[v['outV']] = v['inV']
                elif lbl == 'item':
                    doc = v.get('shard') or v.get('document')
                    for r in v.get('inVs', []):
                        items[v['outV']].append((doc, r))
    return uri_by_doc, range_line, doc_of_range, next_of, defres_of, items


def build_node_index(graph):
    """(rel_file) -> sorted [(line, node_id)] for nearest-enclosing lookup."""
    by_file = defaultdict(list)
    for n in graph['nodes']:
        loc = n.get('source_location') or ''
        sf = (n.get('source_file') or '').replace('\\', '/')
        if not sf or not str(loc).startswith('L'):
            continue
        try:
            line = int(str(loc)[1:]) - 1  # graphify lines are 1-based
        except ValueError:
            continue
        by_file[sf].append((line, n['id']))
    for sf in by_file:
        by_file[sf].sort()
    return by_file


def enclosing(by_file, rel, line):
    """The node in `rel` with the greatest start line <= line."""
    import bisect
    rows = by_file.get(rel)
    if not rows:
        return None
    i = bisect.bisect_right(rows, (line, '￿')) - 1
    return rows[i][1] if i >= 0 else None


def main():
    lsif_path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'graphify-out' / 'index.lsif'
    graph = json.loads(GRAPH.read_text(encoding='utf-8'))
    uri_by_doc, range_line, doc_of_range, next_of, defres_of, items = load_lsif(lsif_path)

    root_uri = ('file:///' + str(ROOT).replace('\\', '/')).lower()

    def rel_of(doc_id):
        uri = uri_by_doc.get(doc_id, '')
        u = uri.replace('\\', '/')
        if u.lower().startswith(root_uri):
            return u[len(root_uri):].lstrip('/')
        return None  # outside the repo (std, deps)

    by_file = build_node_index(graph)
    existing = {(e['source'], e['target']) for e in graph['links']}
    new_edges = []
    seen = set()
    stats = {'refs': 0, 'cross': 0, 'mapped': 0}

    for use_range, result_set in next_of.items():
        defres = defres_of.get(result_set)
        if defres is None:
            continue
        use_doc = doc_of_range.get(use_range)
        use_rel = rel_of(use_doc)
        if use_rel is None:
            continue
        for (def_doc, def_range) in items.get(defres, []):
            stats['refs'] += 1
            def_rel = rel_of(def_doc)
            if def_rel is None or def_rel == use_rel:
                continue
            stats['cross'] += 1
            src = enclosing(by_file, use_rel, range_line.get(use_range, 0))
            dst = enclosing(by_file, def_rel, range_line.get(def_range, 0))
            if not src or not dst or src == dst:
                continue
            key = (src, dst)
            if key in seen or key in existing:
                continue
            seen.add(key)
            stats['mapped'] += 1
            new_edges.append({
                'source': src, 'target': dst, 'relation': RELATION,
                'confidence': 'EXTRACTED', 'confidence_score': 1.0,
                'source_file': use_rel, 'source_location': f'L{range_line.get(use_range, 0) + 1}',
                'weight': 1.0, 'origin': 'rust-analyzer-lsif',
            })

    graph['links'].extend(new_edges)
    GRAPH.write_text(json.dumps(graph, ensure_ascii=False), encoding='utf-8')
    print(f"lsif refs={stats['refs']} cross-file={stats['cross']} "
          f"new unique edges merged={stats['mapped']}; "
          f"graph now {len(graph['nodes'])} nodes / {len(graph['links'])} links")


if __name__ == '__main__':
    main()
