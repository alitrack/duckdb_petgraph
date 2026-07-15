# duckdb_petgraph

DuckDB extension for graph algorithms, powered by the [petgraph](https://github.com/petgraph/petgraph) Rust crate. Run shortest path, connected components, and PageRank directly in SQL — no external graph database required.

## Quickstart

```sql
LOAD 'petgraph_ext.duckdb_extension';

-- 1. Shortest path (Dijkstra)
SELECT * FROM petgraph_shortest_path(
    '[(0,1,2.5),(1,2,1.0),(0,2,5.0)]', 0, 2
);
-- → (path VARCHAR, weight DOUBLE): path='[0,1,2]', weight=3.5

-- 2. Connected components
SELECT * FROM petgraph_connected_components(
    '[(0,1),(1,2),(3,4)]'
);
-- → (node INT, component_id INT)

-- 3. PageRank
SELECT * FROM petgraph_pagerank(
    '[(0,1),(1,2),(2,0),(3,1)]', 0.85, 20
);
-- → (node INT, score DOUBLE)
```

## SQL API

| Function | Parameters | Returns |
|---|---|---|
| `petgraph_shortest_path(edges, from, to)` | edges: `[(src,dst,weight),...]`, from node, to node | (path VARCHAR, weight DOUBLE) |
| `petgraph_connected_components(edges)` | edges: `[(src,dst),...]` | (node INT, component_id INT) |
| `petgraph_pagerank(edges, alpha, iterations)` | edges, damping factor (e.g. 0.85), iterations (e.g. 20) | (node INT, score DOUBLE) |

Edge format: `[(src_node, dst_node, weight), ...]`. Weight is optional — defaults to 1.0. All nodes are typed as INT32.

## Build

```bash
git clone git@github.com:alitrack/duckdb_petgraph.git && cd duckdb_petgraph

# macOS
cargo build --release
python3 scripts/metadata.py target/release/libpetgraph_ext.dylib -o petgraph_ext.duckdb_extension

# Linux
cargo build --release
python3 scripts/metadata.py target/release/libpetgraph_ext.so -o petgraph_ext.duckdb_extension

# Windows
cargo build --release
python3 scripts/metadata.py target/release/petgraph_ext.dll -o petgraph_ext.duckdb_extension
```

## Roadmap

- [x] `petgraph_shortest_path()` — Dijkstra with path reconstruction
- [x] `petgraph_connected_components()` — DFS-based component labeling
- [x] `petgraph_pagerank()` — PageRank with configurable alpha/iterations
- [ ] `petgraph_betweenness_centrality()` — Betweenness centrality
- [ ] `petgraph_strongly_connected_components()` — Tarjan's SCC
- [ ] `petgraph_min_spanning_tree()` — Kruskal's MST
- [ ] Multi-batch streaming for large graphs (>2048 nodes)
- [ ] DuckDB Community Extension submission

## License

MIT — matches petgraph (MIT/Apache-2.0) and duckdb-rs (MIT).
