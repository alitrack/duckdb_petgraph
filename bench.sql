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

-- Build graphs using string_agg
SELECT * FROM petgraph_build_graph('gsmall',
    (SELECT '[' || string_agg('(' || src || ',' || dst || ',' || weight || ')', ',') || ']' FROM edges_small), 1);
SELECT * FROM petgraph_build_graph('gmedium',
    (SELECT '[' || string_agg('(' || src || ',' || dst || ',' || weight || ')', ',') || ']' FROM edges_medium), 1);
SELECT * FROM petgraph_build_graph('glarge',
    (SELECT '[' || string_agg('(' || src || ',' || dst || ',' || weight || ')', ',') || ']' FROM edges_large), 1);

-- ====== Algorithms ======
-- Connected Components
SELECT count(*) FROM petgraph_connected_components('@gsmall');
SELECT count(*) FROM petgraph_connected_components('@gmedium');
SELECT count(*) FROM petgraph_connected_components('@glarge');

-- PageRank
SELECT count(*) FROM petgraph_pagerank('@gsmall', 0.85, 20);
SELECT count(*) FROM petgraph_pagerank('@gmedium', 0.85, 20);
SELECT count(*) FROM petgraph_pagerank('@glarge', 0.85, 20);

-- Shortest Path (from node 0 to node 1)
SELECT * FROM petgraph_shortest_path('@gsmall', 0, 1);
SELECT * FROM petgraph_shortest_path('@gmedium', 0, 1);
SELECT * FROM petgraph_shortest_path('@glarge', 0, 1);

-- SCC
SELECT count(*) FROM petgraph_scc('@gsmall');
SELECT count(*) FROM petgraph_scc('@gmedium');
SELECT count(*) FROM petgraph_scc('@glarge');

-- Betweenness Centrality
SELECT count(*) FROM petgraph_betweenness_centrality('@gsmall');

-- MST (undirected only)
SELECT count(*) FROM petgraph_mst('@gsmall');

-- Cleanup
SELECT * FROM petgraph_drop_graph('gsmall');
SELECT * FROM petgraph_drop_graph('gmedium');
SELECT * FROM petgraph_drop_graph('glarge');
