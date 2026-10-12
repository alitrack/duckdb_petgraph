# duckdb_petgraph

DuckDB extension for graph algorithms, powered by the [petgraph](https://github.com/petgraph/petgraph) Rust crate. Run PageRank, betweenness centrality, closeness centrality, eigenvector centrality, SCC, Louvain community detection, topological sort, cycle detection, shortest path, connected components, and MST directly in SQL — no external graph database required.

## Quickstart

```sql
LOAD 'petgraph_ext.duckdb_extension';

-- 1. Shortest path (A*)
SELECT * FROM petgraph_shortest_path('[(0,1,2.5),(1,2,1.0),(0,2,5.0)]', 0, 2);
-- → (path VARCHAR, weight DOUBLE): path='[0,1,2]', weight=3.5

-- 2. Build a cached graph for repeated queries
SELECT * FROM petgraph_build_graph('my_graph', '[(0,1,2.5),(1,2,1.0)]', 1);
-- 3. Query cached graph by name (zero re-parsing)
SELECT * FROM petgraph_shortest_path('@my_graph', 0, 2);
SELECT * FROM petgraph_pagerank('@my_graph', 0.85, 20);
```

### Feeding data from DuckDB tables

**Method 1: LIST-typed (recommended — zero string serialization)**

```sql
CREATE TABLE edges AS SELECT * FROM (VALUES (0,1,2.5), (1,2,1.0)) t(src, dst, weight);

SET VARIABLE n  = (SELECT count(*) FROM edges);
SET VARIABLE src = (SELECT list(src ORDER BY src) FROM edges);
SET VARIABLE dst = (SELECT list(dst ORDER BY src) FROM edges);
SET VARIABLE wgt = (SELECT list(weight ORDER BY src) FROM edges);

SELECT * FROM petgraph_build_graph_list('g', getvariable('n')::INTEGER,
    getvariable('src'), getvariable('dst'), getvariable('wgt'), 1);
SELECT * FROM petgraph_shortest_path('@g', 0, 2);
```

**Method 2: string_agg (for quick inline use)**

```sql
CREATE TABLE edges AS SELECT * FROM (VALUES (0,1,2.5), (1,2,1.0)) t(src, dst, weight);

SELECT * FROM petgraph_build_graph(
    'g',
    (SELECT '[' || string_agg('(' || src || ',' || dst || ',' || weight || ')', ',') || ']' FROM edges),
    1
);
```

## SQL API

### Data management

| Function | Parameters | Returns |
|---|---|---|
| `petgraph_build_graph(name, edges, directed)` | name, `[(src,dst,weight),...]`, 1=directed | (name, nodes, edges, directed) |
| `petgraph_build_graph_list(name, n, src_list, dst_list, weight_list, directed)` | name, edge count, `LIST<INT>` src, `LIST<INT>` dst, `LIST<FLOAT>` weight, 1=directed | (name, nodes, edges, directed) |
| `petgraph_list_graphs()` | — | (name, nodes, edges, directed) |
| `petgraph_drop_graph(name)` | name | (name, dropped) |
| `petgraph_edges(graph)` | graph (inline or @name) | (src, dst, weight) |
| `petgraph_neighbors(graph, node)` | graph, node (directed only) | (neighbor, weight) |

### Algorithms (all accept inline edges or `@name` for cached graphs)

| Function | Parameters | Returns |
|---|---|---|
| `petgraph_shortest_path(graph, from, to)` | graph, from node, to node | (path VARCHAR, weight DOUBLE) |
| `petgraph_connected_components(graph)` | graph | (node INT, component_id INT) |
| `petgraph_pagerank(graph, alpha, iterations)` | graph, damping factor, iterations | (node INT, score DOUBLE) |
| `petgraph_betweenness_centrality(graph)` | graph (undirected) | (node INT, score DOUBLE) |
| `petgraph_closeness_centrality(graph)` | graph | (node INT, centrality DOUBLE) |
| `petgraph_eigenvector_centrality(graph, iterations, tolerance)` | graph, max_iter, tolerance | (node INT, centrality DOUBLE) |
| `petgraph_scc(graph)` | graph (directed) | (node INT, scc_id INT) |
| `petgraph_louvain(graph)` | graph (directed) | (node INT, community_id INT) |
| `petgraph_toposort(graph)` | graph (directed) | (order INT, node INT) |
| `petgraph_is_cyclic(graph)` | graph (directed) | (is_cyclic INT) |
| `petgraph_mst(graph)` | graph (undirected) | (src INT, dst INT, weight DOUBLE) |

## Build

### Quick build (macOS / Linux)

```bash
git clone git@github.com:alitrack/duckdb_petgraph.git && cd duckdb_petgraph

# macOS
cargo build --release
python3 scripts/metadata.py --extension-version "$(grep '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')" target/release/libpetgraph_ext.dylib -o petgraph_ext.duckdb_extension

# Linux
cargo build --release
python3 scripts/metadata.py --extension-version "$(grep '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')" target/release/libpetgraph_ext.so -o petgraph_ext.duckdb_extension
```

### Makefile build (with extension-ci-tools)

```bash
git clone --recurse-submodules git@github.com:alitrack/duckdb_petgraph.git && cd duckdb_petgraph

make configure    # set up venv, platform, extension version
make release      # build + package as .duckdb_extension
make test_release # run SQLLogicTest suite
```

```sql
duckdb -unsigned -c "LOAD 'build/release/extension/petgraph_ext/petgraph_ext.duckdb_extension'; SELECT * FROM petgraph_shortest_path('[(0,1,2.5),(1,2,1.0),(0,2,5.0)]', 0, 2);"
```

### Testing

```bash
# Run SQLLogicTest suite
make test_release

# Run smoke test (requires duckdb Python package)
pip install duckdb
duckdb -unsigned -c "LOAD 'petgraph_ext.duckdb_extension'; .read test/sql/petgraph.test"
```

## Roadmap

- [x] `petgraph_build_graph()` + `petgraph_list_graphs()` — graph registry (`@name` syntax)
- [x] `petgraph_shortest_path()` — A\* path reconstruction (single traversal)
- [x] `petgraph_connected_components()` — DFS component labeling
- [x] `petgraph_pagerank()` — PageRank with configurable alpha/iterations
- [x] `petgraph_betweenness_centrality()` — Brandes' algorithm (hand-implemented, not in petgraph 0.7)
- [x] `petgraph_scc()` — Tarjan's strongly connected components
- [x] `petgraph_mst()` — Kruskal's minimum spanning tree
- [x] `petgraph_closeness_centrality()` — Closeness centrality (hand-implemented BFS)
- [x] `petgraph_eigenvector_centrality()` — Eigenvector centrality (power iteration)
- [x] `petgraph_louvain()` — Louvain community detection
- [x] `petgraph_toposort()` — Topological sort (Kahns algorithm)
- [x] `petgraph_is_cyclic()` — Directed cycle detection
- [x] Multi-batch streaming — all multi-row functions stream across `func()` calls, no 2048-row ceiling
- [x] `description.yml` — Community Extension submission ready
- [ ] Submit to [duckdb/community-extensions](https://github.com/duckdb/community-extensions)

## License

MIT — matches petgraph (MIT/Apache-2.0) and quack-rs (MIT).
