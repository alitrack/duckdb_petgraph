-- Performance benchmark for duckdb_petgraph
-- Tests build_graph + all algorithms at 3 scales

.timer on

-- ====== Scale 1: Small (1K edges) ======
CREATE TABLE edges_small AS
SELECT (i % 100)::INTEGER AS src, ((i * 7 + 3) % 100)::INTEGER AS dst,
       (1.0 + (i % 10))::FLOAT AS weight
FROM generate_series(1, 1000) t(i);

-- ====== Scale 2: Medium (10K edges) ======
CREATE TABLE edges_medium AS
SELECT (i % 1000)::INTEGER AS src, ((i * 7 + 3) % 1000)::INTEGER AS dst,
       (1.0 + (i % 10))::FLOAT AS weight
FROM generate_series(1, 10000) t(i);

-- ====== Scale 3: Large (100K edges) ======
CREATE TABLE edges_large AS
SELECT (i % 10000)::INTEGER AS src, ((i * 7 + 3) % 10000)::INTEGER AS dst,
       (1.0 + (i % 10))::FLOAT AS weight
FROM generate_series(1, 100000) t(i);

-- Build graphs
SELECT * FROM petgraph_build_graph('gsmall', 'edges_small', 'src', 'dst', 'weight', TRUE);
SELECT * FROM petgraph_build_graph('gmedium', 'edges_medium', 'src', 'dst', 'weight', TRUE);
SELECT * FROM petgraph_build_graph('glarge', 'edges_large', 'src', 'dst', 'weight', TRUE);

-- ====== Algorithms ======
-- Connected Components
SELECT * FROM petgraph_connected_components('gsmall');
SELECT * FROM petgraph_connected_components('gmedium');
SELECT * FROM petgraph_connected_components('glarge');

-- PageRank
SELECT * FROM petgraph_pagerank('gsmall');
SELECT * FROM petgraph_pagerank('gmedium');
SELECT * FROM petgraph_pagerank('glarge');

-- Shortest Path (from node 0)
SELECT * FROM petgraph_shortest_path('gsmall', 0::INTEGER);
SELECT * FROM petgraph_shortest_path('gmedium', 0::INTEGER);
SELECT * FROM petgraph_shortest_path('glarge', 0::INTEGER);

-- SCC
SELECT * FROM petgraph_scc('gsmall');
SELECT * FROM petgraph_scc('gmedium');
SELECT * FROM petgraph_scc('glarge');

-- Betweenness Centrality
SELECT * FROM petgraph_betweenness_centrality('gsmall');
SELECT * FROM petgraph_betweenness_centrality('gmedium');
SELECT * FROM petgraph_betweenness_centrality('glarge');

-- MST (undirected only)
SELECT * FROM petgraph_mst('gsmall');
SELECT * FROM petgraph_mst('gmedium');
SELECT * FROM petgraph_mst('glarge');

-- Cleanup
SELECT * FROM petgraph_drop_graph('gsmall');
SELECT * FROM petgraph_drop_graph('gmedium');
SELECT * FROM petgraph_drop_graph('glarge');
