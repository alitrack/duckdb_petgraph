---
name: duckdb-petgraph-algos
description: >
  Run graph algorithms in DuckDB SQL with the petgraph_ext extension:
  PageRank, betweenness/closeness/eigenvector centrality, Louvain community
  detection, SCC, shortest path (A*), topological sort, cycle detection,
  connected components, MST, triangles, clustering coefficient. Use for
  dependency analysis, fraud rings, network influence, or any edges-table
  workload — no external graph database.
version: 0.5.1
---

# petgraph_ext: 16 graph algorithms in SQL

Extension: `petgraph_ext` (Rust petgraph crate).
`INSTALL petgraph_ext FROM community; LOAD petgraph_ext;`

## Three ways to feed a graph

```sql
-- 1. Inline edge list (ad-hoc): '[(src,dst,weight),...]'
SELECT * FROM petgraph_shortest_path('[(0,1,2.5),(1,2,1.0),(0,2,5.0)]', 0, 2);
-- → path='[0,1,2]', weight=3.5

-- 2. Cached graph by name (repeated queries, zero re-parsing)
SELECT * FROM petgraph_build_graph('my_graph', '[(0,1,2.5),(1,2,1.0)]', 1);  -- 1=directed
SELECT * FROM petgraph_shortest_path('@my_graph', 0, 2);

-- 3. From a DuckDB table — LIST-typed, zero string serialization (recommended)
CREATE TABLE edges AS SELECT * FROM (VALUES (0,1,2.5), (1,2,1.0)) t(src, dst, weight);
SET VARIABLE n   = (SELECT count(*) FROM edges);
SET VARIABLE src = (SELECT list(src ORDER BY src) FROM edges);
SET VARIABLE dst = (SELECT list(dst ORDER BY src) FROM edges);
SET VARIABLE wgt = (SELECT list(weight ORDER BY src) FROM edges);
SELECT * FROM petgraph_build_graph_list('g', getvariable('n')::INTEGER,
    getvariable('src'), getvariable('dst'), getvariable('wgt'), 1);
SELECT * FROM petgraph_shortest_path('@g', 0, 2);
```

DuckDB table functions reject subquery parameters — that's why `SET VARIABLE`
+ `getvariable()` bridges table data into the call.

## Algorithm surface (16)

| Question | Function |
|---|---|
| Who matters most? | `petgraph_pagerank(graph, damping, iters)` |
| Bridge nodes / brokers | `petgraph_betweenness(graph)` |
| Efficient spreaders | `petgraph_closeness(graph)` |
| Influence hubs | `petgraph_eigenvector(graph)` |
| Communities | `petgraph_louvain(graph)` |
| Strongly connected groups | `petgraph_scc(graph)` |
| Route / cost | `petgraph_shortest_path(graph, from, to)` (A*) |
| Build order | `petgraph_toposort(graph)` |
| Deadlock / circular deps | `petgraph_cycle_detect(graph)` |
| Islands | `petgraph_connected_components(graph)` |
| Cheapest wiring | `petgraph_mst(graph)` |
| Triangles | `petgraph_triangles(graph)` |
| Cluster density | `petgraph_clustering(graph)` |

All accept either the inline edge string or `'@name'` cached graphs.

## Graph management

`petgraph_list_graphs()`, `petgraph_drop_graph(name)`, `petgraph_edges(graph)`,
`petgraph_neighbors(graph, node)`.

## Pattern — dependency cycle hunting

```sql
-- edges: (package, depends_on) as integer node ids
SELECT * FROM petgraph_build_graph_list('deps', getvariable('n')::INTEGER,
    getvariable('src'), getvariable('dst'), getvariable('wgt'), 1);
SELECT * FROM petgraph_cycle_detect('@deps');      -- any circular dependency
SELECT * FROM petgraph_toposort('@deps');          -- valid install order
```
