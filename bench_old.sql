.timer on
.echo on

-- BUILD edge strings (string_agg for each scale)
CREATE TABLE bs AS
SELECT 's' as scale, string_agg(format('({},{},{})', src, dst, weight), ',') as edges FROM
  (SELECT (i % 100)::INTEGER AS src, ((i * 7 + 3) % 100)::INTEGER AS dst, (1.0 + (i % 10))::FLOAT AS weight FROM generate_series(1, 1000) t(i));

INSERT INTO bs SELECT 'm', string_agg(format('({},{},{})', src, dst, weight), ',') FROM
  (SELECT (i % 1000)::INTEGER AS src, ((i * 7 + 3) % 1000)::INTEGER AS dst, (1.0 + (i % 10))::FLOAT AS weight FROM generate_series(1, 10000) t(i));

INSERT INTO bs SELECT 'l', string_agg(format('({},{},{})', src, dst, weight), ',') FROM
  (SELECT (i % 10000)::INTEGER AS src, ((i * 7 + 3) % 10000)::INTEGER AS dst, (1.0 + (i % 10))::FLOAT AS weight FROM generate_series(1, 100000) t(i));

-- === OLD BRANCH: edges string passed to every function ===
-- Small (1K)
SELECT * FROM petgraph_build_graph('g1k', '[' || (SELECT edges FROM bs WHERE scale='s') || ']', 1);
SELECT * FROM petgraph_connected_components('[' || (SELECT edges FROM bs WHERE scale='s') || ']');
SELECT * FROM petgraph_pagerank('[' || (SELECT edges FROM bs WHERE scale='s') || ']');
SELECT * FROM petgraph_shortest_path('[' || (SELECT edges FROM bs WHERE scale='s') || ']', 0::INTEGER, 5::INTEGER);
SELECT * FROM petgraph_scc('[' || (SELECT edges FROM bs WHERE scale='s') || ']');
SELECT * FROM petgraph_betweenness_centrality('[' || (SELECT edges FROM bs WHERE scale='s') || ']');
SELECT * FROM petgraph_mst('[' || (SELECT edges FROM bs WHERE scale='s') || ']');

-- Medium (10K)
SELECT * FROM petgraph_build_graph('g10k', '[' || (SELECT edges FROM bs WHERE scale='m') || ']', 1);
SELECT * FROM petgraph_connected_components('[' || (SELECT edges FROM bs WHERE scale='m') || ']');
SELECT * FROM petgraph_pagerank('[' || (SELECT edges FROM bs WHERE scale='m') || ']');
SELECT * FROM petgraph_shortest_path('[' || (SELECT edges FROM bs WHERE scale='m') || ']', 0::INTEGER, 50::INTEGER);
SELECT * FROM petgraph_scc('[' || (SELECT edges FROM bs WHERE scale='m') || ']');
SELECT * FROM petgraph_betweenness_centrality('[' || (SELECT edges FROM bs WHERE scale='m') || ']');
SELECT * FROM petgraph_mst('[' || (SELECT edges FROM bs WHERE scale='m') || ']');

-- Large (100K)
SELECT * FROM petgraph_build_graph('g100k', '[' || (SELECT edges FROM bs WHERE scale='l') || ']', 1);
SELECT * FROM petgraph_connected_components('[' || (SELECT edges FROM bs WHERE scale='l') || ']');
SELECT * FROM petgraph_pagerank('[' || (SELECT edges FROM bs WHERE scale='l') || ']');
SELECT * FROM petgraph_shortest_path('[' || (SELECT edges FROM bs WHERE scale='l') || ']', 0::INTEGER, 500::INTEGER);
SELECT * FROM petgraph_scc('[' || (SELECT edges FROM bs WHERE scale='l') || ']');
SELECT * FROM petgraph_betweenness_centrality('[' || (SELECT edges FROM bs WHERE scale='l') || ']');
SELECT * FROM petgraph_mst('[' || (SELECT edges FROM bs WHERE scale='l') || ']');
