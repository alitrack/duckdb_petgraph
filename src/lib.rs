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
    data::Element,
    graph::{DiGraph, NodeIndex, UnGraph},
    visit::Dfs,
};
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    sync::{Arc, LazyLock, RwLock},
};

// ═══════════════════════════════════════════════
// Global Graph Registry
// ═══════════════════════════════════════════════

enum CachedGraph {
    Directed {
        graph: Arc<DiGraph<(), f64>>,
        id_to_ni: Arc<HashMap<i32, NodeIndex>>,
        ni_to_id: Arc<HashMap<NodeIndex, i32>>,
        node_count: usize,
        edge_count: usize,
    },
    Undirected {
        graph: Arc<UnGraph<(), f64>>,
        id_to_ni: Arc<HashMap<i32, NodeIndex>>,
        ni_to_id: Arc<HashMap<NodeIndex, i32>>,
        node_count: usize,
        edge_count: usize,
    },
}

static GRAPH_REGISTRY: LazyLock<RwLock<HashMap<String, CachedGraph>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

// ═══════════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════════

type EdgeList = Vec<(i32, i32, f64)>;
type DiGraphParts = (
    DiGraph<(), f64>,
    HashMap<i32, NodeIndex>,
    HashMap<NodeIndex, i32>,
);
type UnGraphParts = (
    UnGraph<(), f64>,
    HashMap<i32, NodeIndex>,
    HashMap<NodeIndex, i32>,
);

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
                    // skip
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

fn build_digraph(
    edges: &[(i32, i32, f64)],
) -> (
    DiGraph<(), f64>,
    HashMap<i32, NodeIndex>,
    HashMap<NodeIndex, i32>,
) {
    let mut g = DiGraph::<(), f64>::new();
    let mut id_to_ni: HashMap<i32, NodeIndex> = HashMap::new();
    for (src, dst, w) in edges {
        let u = *id_to_ni.entry(*src).or_insert_with(|| g.add_node(()));
        let v = *id_to_ni.entry(*dst).or_insert_with(|| g.add_node(()));
        g.add_edge(u, v, *w);
    }
    let ni_to_id: HashMap<NodeIndex, i32> = id_to_ni.iter().map(|(k, v)| (*v, *k)).collect();
    (g, id_to_ni, ni_to_id)
}

fn build_undigraph(
    edges: &[(i32, i32, f64)],
) -> (
    UnGraph<(), f64>,
    HashMap<i32, NodeIndex>,
    HashMap<NodeIndex, i32>,
) {
    let mut g = UnGraph::<(), f64>::new_undirected();
    let mut id_to_ni: HashMap<i32, NodeIndex> = HashMap::new();
    for (src, dst, w) in edges {
        let u = *id_to_ni.entry(*src).or_insert_with(|| g.add_node(()));
        let v = *id_to_ni.entry(*dst).or_insert_with(|| g.add_node(()));
        g.add_edge(u, v, *w);
    }
    let ni_to_id: HashMap<NodeIndex, i32> = id_to_ni.iter().map(|(k, v)| (*v, *k)).collect();
    (g, id_to_ni, ni_to_id)
}

/// Resolve a string param to a directed graph — either by parsing edges,
/// or by looking up `@name` in the registry.
fn resolve_digraph(param: &str) -> std::result::Result<DiGraphParts, Box<dyn Error>> {
    if let Some(name) = param.strip_prefix('@') {
        let reg = GRAPH_REGISTRY.read().map_err(|e| e.to_string())?;
        match reg.get(name) {
            Some(CachedGraph::Directed {
                graph,
                id_to_ni,
                ni_to_id,
                ..
            }) => Ok((
                (**graph).clone(),
                (**id_to_ni).clone(),
                (**ni_to_id).clone(),
            )),
            Some(_) => Err(format!("graph '@{name}' is not a directed graph").into()),
            None => Err(format!("graph '@{name}' not found").into()),
        }
    } else {
        let edges = parse_edges(param)?;
        Ok(build_digraph(&edges))
    }
}

fn resolve_undigraph(param: &str) -> std::result::Result<UnGraphParts, Box<dyn Error>> {
    if let Some(name) = param.strip_prefix('@') {
        let reg = GRAPH_REGISTRY.read().map_err(|e| e.to_string())?;
        match reg.get(name) {
            Some(CachedGraph::Undirected {
                graph,
                id_to_ni,
                ni_to_id,
                ..
            }) => Ok((
                (**graph).clone(),
                (**id_to_ni).clone(),
                (**ni_to_id).clone(),
            )),
            Some(_) => Err(format!("graph '@{name}' is not an undirected graph").into()),
            None => Err(format!("graph '@{name}' not found").into()),
        }
    } else {
        let edges = parse_edges(param)?;
        Ok(build_undigraph(&edges))
    }
}

// ═══════════════════════════════════════════════
// petgraph_build_graph(name, edges_str, directed)
// ═══════════════════════════════════════════════

#[repr(C)]
struct BgInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct BgBindData {
    name: String,
    node_count: usize,
    edge_count: usize,
    directed: bool,
}

struct BuildGraphVTab;

impl VTab for BuildGraphVTab {
    type BindData = BgBindData;
    type InitData = BgInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        let pc = bind.get_parameter_count();
        if pc < 3 {
            return Err(
                "petgraph_build_graph: expected 3 params (name, edges_str, directed)".into(),
            );
        }
        let name = bind.get_parameter(0).to_string();
        let edges_str = bind.get_parameter(1).to_string();
        let directed: i32 = parse_int(&bind.get_parameter(2).to_string())?;

        let edges = parse_edges(&edges_str)?;

        let cached = if directed != 0 {
            let (g, id_to_ni, ni_to_id) = build_digraph(&edges);
            let nc = g.node_count();
            let ec = g.edge_count();
            CachedGraph::Directed {
                graph: Arc::new(g),
                id_to_ni: Arc::new(id_to_ni),
                ni_to_id: Arc::new(ni_to_id),
                node_count: nc,
                edge_count: ec,
            }
        } else {
            let (g, id_to_ni, ni_to_id) = build_undigraph(&edges);
            let nc = g.node_count();
            let ec = g.edge_count();
            CachedGraph::Undirected {
                graph: Arc::new(g),
                id_to_ni: Arc::new(id_to_ni),
                ni_to_id: Arc::new(ni_to_id),
                node_count: nc,
                edge_count: ec,
            }
        };

        let nc = match &cached {
            CachedGraph::Directed { node_count, .. } => *node_count,
            CachedGraph::Undirected { node_count, .. } => *node_count,
        };
        let ec = match &cached {
            CachedGraph::Directed { edge_count, .. } => *edge_count,
            CachedGraph::Undirected { edge_count, .. } => *edge_count,
        };

        let mut reg = GRAPH_REGISTRY.write().map_err(|e| e.to_string())?;
        reg.insert(name.clone(), cached);

        bind.add_result_column("name", LogicalTypeHandle::from(LogicalTypeId::Varchar));
        bind.add_result_column("nodes", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("edges", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("directed", LogicalTypeHandle::from(LogicalTypeId::Integer));
        Ok(BgBindData {
            name,
            node_count: nc,
            edge_count: ec,
            directed: directed != 0,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(BgInitData { done: false.into() })
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
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("nodes", DataType::Int32, false),
            Field::new("edges", DataType::Int32, false),
            Field::new("directed", DataType::Int32, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec![bind.name.as_str()])),
                Arc::new(Int32Array::from(vec![bind.node_count as i32])),
                Arc::new(Int32Array::from(vec![bind.edge_count as i32])),
                Arc::new(Int32Array::from(vec![bind.directed as i32])),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![
            LogicalTypeHandle::from(LogicalTypeId::Varchar),
            LogicalTypeHandle::from(LogicalTypeId::Varchar),
            LogicalTypeHandle::from(LogicalTypeId::Integer),
        ])
    }
}

// ═══════════════════════════════════════════════
// petgraph_list_graphs()
// ═══════════════════════════════════════════════

#[repr(C)]
struct LgInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct LgBindData {
    entries: Vec<(String, usize, usize, bool)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct ListGraphsVTab;

impl VTab for ListGraphsVTab {
    type BindData = LgBindData;
    type InitData = LgInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        let reg = GRAPH_REGISTRY.read().map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for (name, g) in reg.iter() {
            match g {
                CachedGraph::Directed {
                    node_count,
                    edge_count,
                    ..
                } => entries.push((name.clone(), *node_count, *edge_count, true)),
                CachedGraph::Undirected {
                    node_count,
                    edge_count,
                    ..
                } => entries.push((name.clone(), *node_count, *edge_count, false)),
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        bind.add_result_column("name", LogicalTypeHandle::from(LogicalTypeId::Varchar));
        bind.add_result_column("nodes", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("edges", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("directed", LogicalTypeHandle::from(LogicalTypeId::Integer));
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("nodes", DataType::Int32, false),
            Field::new("edges", DataType::Int32, false),
            Field::new("directed", DataType::Int32, false),
        ]));
        Ok(LgBindData {
            entries,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(LgInitData { done: false.into() })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.entries.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.entries[offset..offset + n];
        let names: Vec<&str> = slice.iter().map(|e| e.0.as_str()).collect();
        let nodes: Vec<i32> = slice.iter().map(|e| e.1 as i32).collect();
        let edges: Vec<i32> = slice.iter().map(|e| e.2 as i32).collect();
        let directed: Vec<i32> = slice.iter().map(|e| e.3 as i32).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(StringArray::from(names)),
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Int32Array::from(edges)),
                Arc::new(Int32Array::from(directed)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        None
    }
}

// ═══════════════════════════════════════════════
// petgraph_shortest_path(graph, from, to)
// ═══════════════════════════════════════════════

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
            return Err("petgraph_shortest_path: expected 3 params (graph, from, to)".into());
        }
        let (g, id_to_ni, ni_to_id) = resolve_digraph(&bind.get_parameter(0).to_string())?;
        let from: i32 = parse_int(&bind.get_parameter(1).to_string())?;
        let to: i32 = parse_int(&bind.get_parameter(2).to_string())?;

        let from_ni = id_to_ni.get(&from).ok_or(format!(
            "petgraph_shortest_path: node '{from}' not in graph"
        ))?;
        let to_ni = id_to_ni
            .get(&to)
            .ok_or(format!("petgraph_shortest_path: node '{to}' not in graph"))?;

        let dist = algo::dijkstra(&g, *from_ni, Some(*to_ni), |e| *e.weight());
        let total_weight = *dist.get(to_ni).unwrap_or(&f64::INFINITY);
        let mut path_nodes = Vec::new();
        if total_weight.is_finite() {
            let path: Vec<NodeIndex> =
                algo::astar(&g, *from_ni, |n| n == *to_ni, |e| *e.weight(), |_| 0.0)
                    .map(|(_, path)| path)
                    .unwrap_or_default();
            path_nodes = path
                .iter()
                .map(|n| ni_to_id.get(n).copied().unwrap_or(-1))
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
        let schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("weight", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec![path_str])),
                Arc::new(Float64Array::from(vec![bind.total_weight])),
            ],
        )?;
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

// ═══════════════════════════════════════════════
// petgraph_connected_components(graph)
// ═══════════════════════════════════════════════

#[repr(C)]
struct CcInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct CcBindData {
    components: Vec<(i32, i32)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct ConnectedComponentsVTab;

impl VTab for ConnectedComponentsVTab {
    type BindData = CcBindData;
    type InitData = CcInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("petgraph_connected_components: expected 1 param (graph)".into());
        }
        let (g, _, ni_to_id) = resolve_undigraph(&bind.get_parameter(0).to_string())?;

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
                    components.push((ni_to_id.get(&n).copied().unwrap_or(-1), comp_id));
                }
            }
            comp_id += 1;
        }

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column(
            "component_id",
            LogicalTypeHandle::from(LogicalTypeId::Integer),
        );
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("component_id", DataType::Int32, false),
        ]));
        Ok(CcBindData {
            components,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.components.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.components[offset..offset + n];
        let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
        let comps: Vec<i32> = slice.iter().map(|r| r.1).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Int32Array::from(comps)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

// ═══════════════════════════════════════════════
// petgraph_pagerank(graph, alpha, iterations)
// ═══════════════════════════════════════════════

#[repr(C)]
struct PrInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct PrBindData {
    scores: Vec<(i32, f64)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct PageRankVTab;

impl VTab for PageRankVTab {
    type BindData = PrBindData;
    type InitData = PrInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        let pc = bind.get_parameter_count();
        if pc < 3 {
            return Err("petgraph_pagerank: expected 3 params (graph, alpha, iterations)".into());
        }
        let (g, _, ni_to_id) = resolve_digraph(&bind.get_parameter(0).to_string())?;
        let alpha: f64 = bind.get_parameter(1).to_string().parse()?;
        let iterations: usize = bind.get_parameter(2).to_string().parse()?;

        let scores = algo::page_rank(&g, alpha, iterations);
        let mut result: Vec<(i32, f64)> = scores
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let ni = NodeIndex::new(i);
                (ni_to_id.get(&ni).copied().unwrap_or(i as i32), *s)
            })
            .collect();
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("score", LogicalTypeHandle::from(LogicalTypeId::Double));
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("score", DataType::Float64, false),
        ]));
        Ok(PrBindData {
            scores: result,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.scores.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.scores[offset..offset + n];
        let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
        let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Float64Array::from(scores)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
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

// ═══════════════════════════════════════════════
// petgraph_betweenness_centrality(graph)
// ═══════════════════════════════════════════════

#[repr(C)]
struct BcInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct BcBindData {
    scores: Vec<(i32, f64)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct BetweennessCentralityVTab;

impl VTab for BetweennessCentralityVTab {
    type BindData = BcBindData;
    type InitData = BcInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("petgraph_betweenness_centrality: expected 1 param (graph)".into());
        }
        let (g, _, ni_to_id) = resolve_undigraph(&bind.get_parameter(0).to_string())?;

        // Brandes' algorithm for betweenness centrality (not in petgraph 0.7)
        let n = g.node_count();
        let mut cb = vec![0.0f64; n];
        for s in g.node_indices() {
            let mut stack = Vec::new();
            let mut pred: Vec<Vec<NodeIndex>> = vec![Vec::new(); n];
            let mut sigma = vec![0u64; n];
            let mut dist = vec![-1i32; n];
            let mut q = VecDeque::new();
            sigma[s.index()] = 1;
            dist[s.index()] = 0;
            q.push_back(s);
            while let Some(v) = q.pop_front() {
                stack.push(v);
                for w in g.neighbors(v) {
                    if dist[w.index()] < 0 {
                        dist[w.index()] = dist[v.index()] + 1;
                        q.push_back(w);
                    }
                    if dist[w.index()] == dist[v.index()] + 1 {
                        sigma[w.index()] += sigma[v.index()];
                        pred[w.index()].push(v);
                    }
                }
            }
            let mut delta = vec![0.0f64; n];
            while let Some(w) = stack.pop() {
                for &v in &pred[w.index()] {
                    delta[v.index()] += (sigma[v.index()] as f64 / sigma[w.index()] as f64)
                        * (1.0 + delta[w.index()]);
                }
                if w != s {
                    cb[w.index()] += delta[w.index()];
                }
            }
        }
        let mut scores: Vec<(i32, f64)> = cb
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let ni = NodeIndex::new(i);
                (ni_to_id.get(&ni).copied().unwrap_or(i as i32), s)
            })
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("score", LogicalTypeHandle::from(LogicalTypeId::Double));
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("score", DataType::Float64, false),
        ]));
        Ok(BcBindData {
            scores,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(BcInitData { done: false.into() })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.scores.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.scores[offset..offset + n];
        let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
        let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Float64Array::from(scores)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

// ═══════════════════════════════════════════════
// petgraph_scc(graph) — Tarjan's strongly connected components
// ═══════════════════════════════════════════════

#[repr(C)]
struct SccInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct SccBindData {
    components: Vec<(i32, i32)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct SccVTab;

impl VTab for SccVTab {
    type BindData = SccBindData;
    type InitData = SccInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("petgraph_scc: expected 1 param (graph)".into());
        }
        let (g, _, ni_to_id) = resolve_digraph(&bind.get_parameter(0).to_string())?;

        let sccs = algo::tarjan_scc(&g);
        let mut components = Vec::new();
        for (comp_id, scc) in sccs.iter().enumerate() {
            for n in scc {
                components.push((ni_to_id.get(n).copied().unwrap_or(-1), comp_id as i32));
            }
        }

        bind.add_result_column("node", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("scc_id", LogicalTypeHandle::from(LogicalTypeId::Integer));
        let schema = Arc::new(Schema::new(vec![
            Field::new("node", DataType::Int32, false),
            Field::new("scc_id", DataType::Int32, false),
        ]));
        Ok(SccBindData {
            components,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(SccInitData { done: false.into() })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.components.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.components[offset..offset + n];
        let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
        let comps: Vec<i32> = slice.iter().map(|r| r.1).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(Int32Array::from(nodes)),
                Arc::new(Int32Array::from(comps)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

// ═══════════════════════════════════════════════
// petgraph_mst(graph) — Kruskal's minimum spanning tree
// ═══════════════════════════════════════════════

#[repr(C)]
struct MstInitData {
    done: std::sync::atomic::AtomicBool,
}

#[repr(C)]
struct MstBindData {
    edges: Vec<(i32, i32, f64)>,
    offset: std::sync::atomic::AtomicUsize,
    schema: Arc<arrow::datatypes::Schema>,
}

struct MstVTab;

impl VTab for MstVTab {
    type BindData = MstBindData;
    type InitData = MstInitData;

    fn bind(bind: &BindInfo) -> std::result::Result<Self::BindData, Box<dyn Error>> {
        if bind.get_parameter_count() < 1 {
            return Err("petgraph_mst: expected 1 param (graph)".into());
        }
        let (g, _, ni_to_id) = resolve_undigraph(&bind.get_parameter(0).to_string())?;

        let mst_edges = algo::min_spanning_tree(&g);
        let edges: Vec<(i32, i32, f64)> = mst_edges
            .filter_map(|el| match el {
                Element::Edge {
                    source,
                    target,
                    weight,
                } => {
                    let s = NodeIndex::new(source);
                    let t = NodeIndex::new(target);
                    Some((
                        ni_to_id.get(&s).copied().unwrap_or(-1),
                        ni_to_id.get(&t).copied().unwrap_or(-1),
                        weight,
                    ))
                }
                Element::Node { .. } => None,
            })
            .collect();

        bind.add_result_column("src", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("dst", LogicalTypeHandle::from(LogicalTypeId::Integer));
        bind.add_result_column("weight", LogicalTypeHandle::from(LogicalTypeId::Double));
        let schema = Arc::new(Schema::new(vec![
            Field::new("src", DataType::Int32, false),
            Field::new("dst", DataType::Int32, false),
            Field::new("weight", DataType::Float64, false),
        ]));
        Ok(MstBindData {
            edges,
            offset: std::sync::atomic::AtomicUsize::new(0),
            schema,
        })
    }

    fn init(_: &InitInfo) -> std::result::Result<Self::InitData, Box<dyn Error>> {
        Ok(MstInitData { done: false.into() })
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
        let offset = bind.offset.load(std::sync::atomic::Ordering::Relaxed);
        let remaining = bind.edges.len().saturating_sub(offset);
        let n = remaining.min(2048);
        if n == 0 {
            init.done.store(true, std::sync::atomic::Ordering::Relaxed);
            output.set_len(0);
            return Ok(());
        }
        let slice = &bind.edges[offset..offset + n];
        let srcs: Vec<i32> = slice.iter().map(|e| e.0).collect();
        let dsts: Vec<i32> = slice.iter().map(|e| e.1).collect();
        let weights: Vec<f64> = slice.iter().map(|e| e.2).collect();
        let batch = RecordBatch::try_new(
            bind.schema.clone(),
            vec![
                Arc::new(Int32Array::from(srcs)),
                Arc::new(Int32Array::from(dsts)),
                Arc::new(Float64Array::from(weights)),
            ],
        )?;
        record_batch_to_duckdb_data_chunk(&batch, output)?;
        bind.offset
            .store(offset + n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

// ═══════════════════════════════════════════════
// Entrypoint
// ═══════════════════════════════════════════════

#[duckdb_entrypoint_c_api(ext_name = "petgraph_ext")]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    con.register_table_function::<BuildGraphVTab>("petgraph_build_graph")?;
    con.register_table_function::<ListGraphsVTab>("petgraph_list_graphs")?;
    con.register_table_function::<ShortestPathVTab>("petgraph_shortest_path")?;
    con.register_table_function::<ConnectedComponentsVTab>("petgraph_connected_components")?;
    con.register_table_function::<PageRankVTab>("petgraph_pagerank")?;
    con.register_table_function::<BetweennessCentralityVTab>("petgraph_betweenness_centrality")?;
    con.register_table_function::<SccVTab>("petgraph_scc")?;
    con.register_table_function::<MstVTab>("petgraph_mst")?;
    Ok(())
}
