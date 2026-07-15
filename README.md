# duckdb_petgraph

DuckDB extension for graph algorithms, powered by the [petgraph](https://github.com/petgraph/petgraph) Rust crate. Run PageRank, betweenness centrality, closeness centrality, eigenvector centrality, SCC, Louvain community detection, topological sort, cycle detection, shortest path, connected components, and MST directly in SQL — no external graph database required.

## Quickstart

```sql
LOAD 'petgraph_ext.duckdb_extension';

-- 1. Shortest path (Dijkstra + A*)
SELECT * FROM petgraph_shortest_path('[(0,1,2.5),(1,2,1.0),(0,2,5.0)]', 0, 2);
-- → (path VARCHAR, weight DOUBLE): path='[0,1,2]', weight=3.5

-- 2. Build a cached graph for repeated queries
SELECT * FROM petgraph_build_graph('my_graph', '[(0,1,2.5),(1,2,1.0)]', 1);
-- 3. Query cached graph by name (zero re-parsing)
SELECT * FROM petgraph_shortest_path('@my_graph', 0, 2);
SELECT * FROM petgraph_pagerank('@my_graph', 0.85, 20);
```

### Feeding data from DuckDB tables

```sql
CREATE TABLE edges AS SELECT * FROM (VALUES (0,1,2.5), (1,2,1.0)) t(src, dst, weight);

-- Serialize → build → cache
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
| `petgraph_louvain(graph)` | graph | (node INT, community_id INT) |
| `petgraph_toposort(graph)` | graph (directed) | (order INT, node INT) |
| `petgraph_is_cyclic(graph)` | graph (directed) | (is_cyclic INT) |
| `petgraph_mst(graph)` | graph (undirected) | (src INT, dst INT, weight DOUBLE) |

## Build

```bash
git clone git@github.com:alitrack/duckdb_petgraph.git && cd duckdb_petgraph

# macOS
cargo build --release
python3 scripts/metadata.py target/release/libpetgraph_ext.dylib -o petgraph_ext.duckdb_extension

# Linux
cargo build --release
python3 scripts/metadata.py target/release/libpetgraph_ext.so -o petgraph_ext.duckdb_extension
```

## Roadmap

- [x] `petgraph_build_graph()` + `petgraph_list_graphs()` — graph registry (`@name` syntax)
- [x] `petgraph_shortest_path()` — Dijkstra with A\* path reconstruction
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
- [ ] DuckDB Community Extension submission

## License

MIT — matches petgraph (MIT/Apache-2.0) and duckdb-rs (MIT).
