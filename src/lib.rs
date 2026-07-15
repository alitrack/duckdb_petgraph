use arrow::{
    array::{Float64Array, Int32Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use duckdb::{
    core::{DataChunkHandle, LogicalTypeHandle, LogicalTypeId},
    duckdb_entrypoint_c_api,
    vtab::{arrow::record_batch_to_duckdb_data_chunk, BindInfo, InitInfo, TableFunctionInfo, VTab},
    Connection, Result,
};
use petgraph::{
    algo,
    graph::{DiGraph, UnGraph},
    visit::Dfs,
};
use std::{collections::HashMap, error::Error, sync::Arc};

// ── Shared utilities ──

type EdgeList = Vec<(i32, i32, f64)>;

/// Parse edges string: "[(0,1,1.0),(1,2,2.5),...]" → Vec<(i32, i32, f64)>
fn parse_edges(raw: &str) -> std::result::Result<EdgeList, Box<dyn Error>> {
    let s = raw.trim();
    if !s.starts_with('[') || !s.ends_with(']') {
        return Err("edges: expected [...] format".into());
    }
    let inner = &s[1..s.len() - 1];
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut edges = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                if depth == 0 {
                    current.clear();
                }
                depth += 1;
                if depth > 1 {
                    current.push(ch);
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    // Parse "(src,dst,weight)" or "(src,dst)"
                    let cleaned = current.trim();
                    let parts: Vec<&str> = cleaned.splitn(3, ',').collect();
                    if parts.len() < 2 {
                        return Err(
                            format!("edge: expected (src,dst[,weight]), got ({cleaned})").into(),
                        );
                    }
                    let src: i32 = parts[0].trim().parse()?;
                    let dst: i32 = parts[1].trim().parse()?;
                    let weight: f64 = if parts.len() >= 3 {
                        parts[2].trim().parse()?
                    } else {
                        1.0
                    };
                    edges.push((src, dst, weight));
                } else {
                    current.push(ch);
                }
            }
            ',' => {
                if depth == 0 {
                    // separator between edges, skip
                } else {
                    current.push(ch);
                }
            }
            _ => current.push(ch),
        }
    }
    Ok(edges)
}

fn parse_int(raw: &str) -> std::result::Result<i32, Box<dyn Error>> {
    Ok(raw.trim().parse()?)
}

// ── petgraph_shortest_path(edges_str, from, to) ──

#[repr(C)]
struct SpInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct SpBindData {
    path_nodes: Vec<i32>,
    total_weight: f64,
}

struct ShortestPathVTab;

impl VTab for ShortestPathVTab {
    type BindData = SpBindData;
    type InitData = SpInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        let pc = bind.get_parameter_count();
        if pc < 3 {
            return Err("petgraph_shortest_path: expected 3 params (edges_str, from, to)".into());
        }
        let edges = parse_edges(&bind.get_parameter(0).to_string())?;
        let from: i32 = parse_int(&bind.get_parameter(1).to_string())?;
        let to: i32 = parse_int(&bind.get_parameter(2).to_string())?;

        // Build directed graph
        let mut g = DiGraph::<(), f64>::new();
        let mut node_map: HashMap<i32, petgraph::graph::NodeIndex> = HashMap::new();
        let mut get_or_insert = |g: &mut DiGraph<(), f64>, id: i32| -> petgraph::graph::NodeIndex {
            *node_map.entry(id).or_insert_with(|| g.add_node(()))
        };
        for (src, dst, w) in &edges {
            let u = get_or_insert(&mut g, *src);
            let v = get_or_insert(&mut g, *dst);
            g.add_edge(u, v, *w);
        }

        let from_ni = node_map.get(&from).ok_or(format!(
            "petgraph_shortest_path: node '{from}' not in graph"
        ))?;
        let to_ni = node_map
            .get(&to)
            .ok_or(format!("petgraph_shortest_path: node '{to}' not in graph"))?;

        // Reverse mapping for result
        let rev_map: HashMap<petgraph::graph::NodeIndex, i32> =
            node_map.iter().map(|(k, v)| (*v, *k)).collect();

        let path = algo::dijkstra(&g, *from_ni, Some(*to_ni), |e| *e.weight());

        let total_weight = *path.get(to_ni).unwrap_or(&f64::INFINITY);
        let mut path_nodes = Vec::new();
        if total_weight.is_finite() {
            // Reconstruct path by backtracking
            let path: Vec<petgraph::graph::NodeIndex> =
                algo::astar(&g, *from_ni, |n| n == *to_ni, |e| *e.weight(), |_| 0.0)
                    .map(|(_, path)| path)
                    .unwrap_or_default();
            path_nodes = path
                .iter()
                .map(|n| rev_map.get(n).copied().unwrap_or(-1))
                .collect();
        }

        bind.add_result_column("path", LogicalTypeHandle::from(LogicalTypeId::Varchar));
        bind.add_result_column("weight", LogicalTypeHandle::from(LogicalTypeId::Double));
        Ok(SpBindData {
            path_nodes,
            total_weight,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(SpInitData { done: false.into() })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> std::result::Result<(), Box<dyn Error>> {
        let init = func.get_init_data();
        if init.done.load(std::sync::atomic::Ordering::Relaxed) {
            output.set_len(0);
            return Ok(());
        }
        init.done.store(true, std::sync::atomic::Ordering::Relaxed);
        let bind = func.get_bind_data();

        // Return one row with path as JSON array string and total weight
        let path_str = if bind.path_nodes.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                bind.path_nodes
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        // Register path column in the output schema
        let schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("weight", DataType::Float64, false),
        ]));
        let path_arr = StringArray::from(vec![path_str]);
        let weight_arr = Float64Array::from(vec![bind.total_weight]);
        let batch = RecordBatch::try_new(schema, vec![Arc::new(path_arr), Arc::new(weight_arr)])?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![
            LogicalTypeHandle::from(LogicalTypeId::Varchar),
            LogicalTypeHandle::from(LogicalTypeId::Integer),
            LogicalTypeHandle::from(LogicalTypeId::Integer),
        ])
    }
}

// ── petgraph_connected_components(edges_str) ──

#[repr(C)]
struct CcInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct CcBindData {
    components: Vec<(i32, i32)>, // (node, component_id)
}

struct ConnectedComponentsVTab;

impl VTab for ConnectedComponentsVTab {
    type BindData = CcBindData;
    type InitData = CcInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("petgraph_connected_components: expected 1 param (edges_str)".into());
        }
        let edges = parse_edges(&bind.get_parameter(0).to_string())?;

        // Build undirected graph
        let mut g = UnGraph::<(), ()>::new_undirected();
        let mut node_map: HashMap<i32, petgraph::graph::NodeIndex> = HashMap::new();
        let mut get_or_insert = |g: &mut UnGraph<(), ()>, id: i32| -> petgraph::graph::NodeIndex {
            *node_map.entry(id).or_insert_with(|| g.add_node(()))
        };
        for (src, dst, _) in &edges {
            let u = get_or_insert(&mut g, *src);
            let v = get_or_insert(&mut g, *dst);
            g.add_edge(u, v, ());
        }
        let rev_map: HashMap<petgraph::graph::NodeIndex, i32> =
            node_map.iter().map(|(k, v)| (*v, *k)).collect();

        // Connected components via DFS
        let mut components = Vec::new();
        let mut visited = vec![false; g.node_count()];
        let mut comp_id = 0;
        for start in g.node_indices() {
            if visited[start.index()] {
                continue;
            }
            let mut dfs = Dfs::new(&g, start);
            while let Some(n) = dfs.next(&g) {
                if !visited[n.index()] {
                    visited[n.index()] = true;
                    components.push((rev_map.get(&n).copied().unwrap_or(-1), comp_id));
                }
            }
            comp_id += 1;
        }

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column(
            "component_id",
            LogicalTypeHandle::from(LogicalTypeId::Integer),
        );
        Ok(CcBindData { components })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(CcInitData { done: false.into() })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> std::result::Result<(), Box<dyn Error>> {
        let init = func.get_init_data();
        if init.done.load(std::sync::atomic::Ordering::Relaxed) {
            output.set_len(0);
            return Ok(());
        }
        let bind = func.get_bind_data();
        let n = bind.components.len().min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let nodes: Vec<i32> = bind.components.iter().take(n).map(|r| r.0).collect();
        let comps: Vec<i32> = bind.components.iter().take(n).map(|r| r.1).collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("component_id", DataType::Int32, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Int32Array::from(comps)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        init.done.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

// ── petgraph_pagerank(edges_str, alpha, iterations) ──

#[repr(C)]
struct PrInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct PrBindData {
    scores: Vec<(i32, f64)>, // (node, score)
}

struct PageRankVTab;

impl VTab for PageRankVTab {
    type BindData = PrBindData;
    type InitData = PrInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        let pc = bind.get_parameter_count();
        if pc < 1 {
            return Err(
                "petgraph_pagerank: expected at least 1 param (edges_str, [alpha, iterations])"
                    .into(),
            );
        }
        let edges = parse_edges(&bind.get_parameter(0).to_string())?;
        let alpha: f64 = if pc >= 2 {
            bind.get_parameter(1).to_string().parse()?
        } else {
            0.85
        };
        let iterations: usize = if pc >= 3 {
            bind.get_parameter(2).to_string().parse()?
        } else {
            20
        };

        let mut g = DiGraph::<(), f64>::new();
        let mut node_map: HashMap<i32, petgraph::graph::NodeIndex> = HashMap::new();
        let mut get_or_insert = |g: &mut DiGraph<(), f64>, id: i32| -> petgraph::graph::NodeIndex {
            *node_map.entry(id).or_insert_with(|| g.add_node(()))
        };
        for (src, dst, w) in &edges {
            let u = get_or_insert(&mut g, *src);
            let v = get_or_insert(&mut g, *dst);
            g.add_edge(u, v, *w);
        }
        let rev_map: HashMap<petgraph::graph::NodeIndex, i32> =
            node_map.iter().map(|(k, v)| (*v, *k)).collect();

        let scores = algo::page_rank(&g, alpha, iterations);
        let mut result: Vec<(i32, f64)> = scores
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let ni = petgraph::graph::NodeIndex::new(i);
                (rev_map.get(&ni).copied().unwrap_or(i as i32), *s)
            })
            .collect();
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("score", LogicalTypeHandle::from(LogicalTypeId::Double));
        Ok(PrBindData { scores: result })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(PrInitData { done: false.into() })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> std::result::Result<(), Box<dyn Error>> {
        let init = func.get_init_data();
        if init.done.load(std::sync::atomic::Ordering::Relaxed) {
            output.set_len(0);
            return Ok(());
        }
        let bind = func.get_bind_data();
        let n = bind.scores.len().min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let nodes: Vec<i32> = bind.scores.iter().take(n).map(|r| r.0).collect();
        let scores: Vec<f64> = bind.scores.iter().take(n).map(|r| r.1).collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("score", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Float64Array::from(scores)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        init.done.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![
            LogicalTypeHandle::from(LogicalTypeId::Varchar),
            LogicalTypeHandle::from(LogicalTypeId::Double),
            LogicalTypeHandle::from(LogicalTypeId::Integer),
        ])
    }
}

// ── Entrypoint ──

#[duckdb_entrypoint_c_api(ext_name = "petgraph_ext")]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    con.register_table_function::<ShortestPathVTab>("petgraph_shortest_path")?;
    con.register_table_function::<ConnectedComponentsVTab>("petgraph_connected_components")?;
    con.register_table_function::<PageRankVTab>("petgraph_pagerank")?;
    Ok(())
}
