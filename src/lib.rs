use petgraph::{
    algo,
    data::Element,
    graph::{DiGraph, NodeIndex, UnGraph},
    visit::{Dfs, EdgeRef},
};
use quack_rs::{
    prelude::*,
    table::TableFunctionBuilder,
    types::TypeId,
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
type DiGraphParts = (DiGraph<(), f64>, HashMap<i32, NodeIndex>, HashMap<NodeIndex, i32>);
type UnGraphParts = (UnGraph<(), f64>, HashMap<i32, NodeIndex>, HashMap<NodeIndex, i32>);

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
                if depth == 0 { current.clear(); }
                depth += 1;
                if depth > 1 { current.push(ch); }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let cleaned = current.trim();
                    let parts: Vec<&str> = cleaned.splitn(3, ',').collect();
                    if parts.len() < 2 {
                        return Err(format!("edge: expected (src,dst[,weight]), got ({cleaned})").into());
                    }
                    let src: i32 = parts[0].trim().parse()?;
                    let dst: i32 = parts[1].trim().parse()?;
                    let weight: f64 = if parts.len() >= 3 { parts[2].trim().parse()? } else { 1.0 };
                    edges.push((src, dst, weight));
                } else { current.push(ch); }
            }
            ',' => { if depth != 0 { current.push(ch); } }
            _ => current.push(ch),
        }
    }
    Ok(edges)
}

fn build_digraph(edges: &[(i32, i32, f64)]) -> DiGraphParts {
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

fn build_undigraph(edges: &[(i32, i32, f64)]) -> UnGraphParts {
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

fn resolve_digraph(param: &str) -> std::result::Result<DiGraphParts, Box<dyn Error>> {
    if let Some(name) = param.strip_prefix('@') {
        let reg = GRAPH_REGISTRY.read().map_err(|e| e.to_string())?;
        match reg.get(name) {
            Some(CachedGraph::Directed { graph, id_to_ni, ni_to_id, .. }) => Ok((
                (**graph).clone(), (**id_to_ni).clone(), (**ni_to_id).clone(),
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
            Some(CachedGraph::Undirected { graph, id_to_ni, ni_to_id, .. }) => Ok((
                (**graph).clone(), (**id_to_ni).clone(), (**ni_to_id).clone(),
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
// FFI helpers for writing to DuckDB vectors
// ═══════════════════════════════════════════════

use libduckdb_sys::{
    duckdb_data_chunk, duckdb_data_chunk_get_vector, duckdb_data_chunk_set_size,
    duckdb_vector_assign_string_element_len, duckdb_vector_get_data, idx_t,
};

unsafe fn write_i32_col(chunk: duckdb_data_chunk, col: usize, data: &[i32], n: usize) {
    let vec = duckdb_data_chunk_get_vector(chunk, col as idx_t);
    let ptr = duckdb_vector_get_data(vec).cast::<i32>();
    for (i, &v) in data.iter().enumerate().take(n) {
        *ptr.add(i) = v;
    }
}

unsafe fn write_f64_col(chunk: duckdb_data_chunk, col: usize, data: &[f64], n: usize) {
    let vec = duckdb_data_chunk_get_vector(chunk, col as idx_t);
    let ptr = duckdb_vector_get_data(vec).cast::<f64>();
    for (i, &v) in data.iter().enumerate().take(n) {
        *ptr.add(i) = v;
    }
}

unsafe fn write_varchar_col(chunk: duckdb_data_chunk, col: usize, data: &[String], n: usize) {
    let vec = duckdb_data_chunk_get_vector(chunk, col as idx_t);
    for (i, s) in data.iter().enumerate().take(n) {
        duckdb_vector_assign_string_element_len(
            vec,
            i as idx_t,
            s.as_ptr().cast::<std::os::raw::c_char>(),
            s.len() as idx_t,
        );
    }
}

// ═══════════════════════════════════════════════
// 1. petgraph_build_graph
// ═══════════════════════════════════════════════

struct BuildGraphState { name: String, node_count: usize, edge_count: usize, directed: bool, done: bool }

fn build_petgraph_build_graph() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_build_graph")
        .param(TypeId::Varchar).param(TypeId::Varchar).param(TypeId::Integer)
        .with_state::<BuildGraphState, _>(|bind| {
            if bind.parameter_count() < 3 {
                return Err(ExtensionError::new("petgraph_build_graph: expected 3 params (name, edges_str, directed)"));
            }
            let name = unsafe { bind.get_parameter_value(0) }.as_str_or("").to_string();
            let edges_str = unsafe { bind.get_parameter_value(1) }.as_str_or("").to_string();
            let directed = unsafe { bind.get_parameter_value(2) }.as_i64_or(0) != 0;
            let edges = parse_edges(&edges_str).map_err(|e| ExtensionError::new(&e.to_string()))?;

            let (nc, ec, cached) = if directed {
                let (g, id_to_ni, ni_to_id) = build_digraph(&edges);
                let nc = g.node_count();
                let ec = g.edge_count();
                (nc, ec, CachedGraph::Directed { graph: Arc::new(g), id_to_ni: Arc::new(id_to_ni), ni_to_id: Arc::new(ni_to_id), node_count: nc, edge_count: ec })
            } else {
                let (g, id_to_ni, ni_to_id) = build_undigraph(&edges);
                let nc = g.node_count();
                let ec = g.edge_count();
                (nc, ec, CachedGraph::Undirected { graph: Arc::new(g), id_to_ni: Arc::new(id_to_ni), ni_to_id: Arc::new(ni_to_id), node_count: nc, edge_count: ec })
            };
            let mut reg = GRAPH_REGISTRY.write().map_err(|e| ExtensionError::new(&e.to_string()))?;
            reg.insert(name.clone(), cached);
            bind.add_result_column("name", TypeId::Varchar);
            bind.add_result_column("nodes", TypeId::Integer);
            bind.add_result_column("edges", TypeId::Integer);
            bind.add_result_column("directed", TypeId::Integer);
            Ok(BuildGraphState { name, node_count: nc, edge_count: ec, directed, done: false })
        })
        .scan(|state, chunk| {
            if state.done { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            state.done = true;
            let raw = unsafe { chunk.as_raw() };
            unsafe { write_varchar_col(raw, 0, &[state.name.clone()], 1) };
            unsafe { write_i32_col(raw, 1, &[state.node_count as i32], 1) };
            unsafe { write_i32_col(raw, 2, &[state.edge_count as i32], 1) };
            unsafe { write_i32_col(raw, 3, &[state.directed as i32], 1) };
            unsafe { duckdb_data_chunk_set_size(raw, 1) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 2. petgraph_list_graphs
// ═══════════════════════════════════════════════

struct ListGraphsState { entries: Vec<(String, usize, usize, bool)>, offset: usize }

fn build_petgraph_list_graphs() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_list_graphs")
        .with_state::<ListGraphsState, _>(|bind| {
            let reg = GRAPH_REGISTRY.read().map_err(|e| ExtensionError::new(&e.to_string()))?;
            let mut entries = Vec::new();
            for (name, g) in reg.iter() {
                match g {
                    CachedGraph::Directed { node_count, edge_count, .. } => entries.push((name.clone(), *node_count, *edge_count, true)),
                    CachedGraph::Undirected { node_count, edge_count, .. } => entries.push((name.clone(), *node_count, *edge_count, false)),
                }
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            bind.add_result_column("name", TypeId::Varchar);
            bind.add_result_column("nodes", TypeId::Integer);
            bind.add_result_column("edges", TypeId::Integer);
            bind.add_result_column("directed", TypeId::Integer);
            Ok(ListGraphsState { entries, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.entries.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.entries[state.offset..state.offset + n];
            let names: Vec<String> = slice.iter().map(|e| e.0.clone()).collect();
            let nodes: Vec<i32> = slice.iter().map(|e| e.1 as i32).collect();
            let edges: Vec<i32> = slice.iter().map(|e| e.2 as i32).collect();
            let directed: Vec<i32> = slice.iter().map(|e| e.3 as i32).collect();
            unsafe { write_varchar_col(raw, 0, &names, n) };
            unsafe { write_i32_col(raw, 1, &nodes, n) };
            unsafe { write_i32_col(raw, 2, &edges, n) };
            unsafe { write_i32_col(raw, 3, &directed, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 3. petgraph_drop_graph
// ═══════════════════════════════════════════════

struct DropGraphState { name: String, existed: bool, done: bool }

fn build_petgraph_drop_graph() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_drop_graph")
        .param(TypeId::Varchar)
        .with_state::<DropGraphState, _>(|bind| {
            if bind.parameter_count() < 1 {
                return Err(ExtensionError::new("petgraph_drop_graph: expected 1 param (name)"));
            }
            let name = unsafe { bind.get_parameter_value(0) }.as_str_or("").to_string();
            let mut reg = GRAPH_REGISTRY.write().map_err(|e| ExtensionError::new(&e.to_string()))?;
            let existed = reg.remove(&name).is_some();
            bind.add_result_column("name", TypeId::Varchar);
            bind.add_result_column("dropped", TypeId::Integer);
            Ok(DropGraphState { name, existed, done: false })
        })
        .scan(|state, chunk| {
            if state.done { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            state.done = true;
            let raw = unsafe { chunk.as_raw() };
            unsafe { write_varchar_col(raw, 0, &[state.name.clone()], 1) };
            unsafe { write_i32_col(raw, 1, &[state.existed as i32], 1) };
            unsafe { duckdb_data_chunk_set_size(raw, 1) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 4. petgraph_shortest_path
// ═══════════════════════════════════════════════

struct SpState { path: String, weight: f64, done: bool }

fn build_petgraph_shortest_path() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_shortest_path")
        .param(TypeId::Varchar).param(TypeId::Integer).param(TypeId::Integer)
        .with_state::<SpState, _>(|bind| {
            if bind.parameter_count() < 3 {
                return Err(ExtensionError::new("petgraph_shortest_path: expected 3 params (graph, from, to)"));
            }
            let graph_str = unsafe { bind.get_parameter_value(0) }.as_str_or("").to_string();
            let from = unsafe { bind.get_parameter_value(1) }.as_i64_or(0) as i32;
            let to = unsafe { bind.get_parameter_value(2) }.as_i64_or(0) as i32;
            let (g, id_to_ni, ni_to_id) = resolve_digraph(&graph_str).map_err(|e| ExtensionError::new(&e.to_string()))?;

            let from_ni = *id_to_ni.get(&from).ok_or_else(|| ExtensionError::new(&format!("node '{from}' not in graph")))?;
            let to_ni = *id_to_ni.get(&to).ok_or_else(|| ExtensionError::new(&format!("node '{to}' not in graph")))?;

            let (total_weight, path) = match algo::astar(&g, from_ni, |n| n == to_ni, |e| *e.weight(), |_| 0.0) {
                Some((cost, nodes)) => {
                    let ids: Vec<String> = nodes.iter().map(|n| ni_to_id.get(n).copied().unwrap_or(-1).to_string()).collect();
                    (cost, format!("[{}]", ids.join(",")))
                }
                None => (f64::INFINITY, "[]".to_string()),
            };
            bind.add_result_column("path", TypeId::Varchar);
            bind.add_result_column("weight", TypeId::Double);
            Ok(SpState { path, weight: total_weight, done: false })
        })
        .scan(|state, chunk| {
            if state.done { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            state.done = true;
            let raw = unsafe { chunk.as_raw() };
            unsafe { write_varchar_col(raw, 0, &[state.path.clone()], 1) };
            unsafe { write_f64_col(raw, 1, &[state.weight], 1) };
            unsafe { duckdb_data_chunk_set_size(raw, 1) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 5. petgraph_connected_components
// ═══════════════════════════════════════════════

struct CcState { data: Vec<(i32, i32)>, offset: usize }

fn build_petgraph_connected_components() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_connected_components")
        .param(TypeId::Varchar)
        .with_state::<CcState, _>(|bind| {
            let param = unsafe { bind.get_parameter_value(0) }.as_str_or("").to_string();
            let (g, _, ni_to_id) = resolve_undigraph(&param).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let mut data = Vec::new();
            let mut visited = vec![false; g.node_count()];
            let mut comp_id = 0;
            for start in g.node_indices() {
                if visited[start.index()] { continue; }
                let mut dfs = Dfs::new(&g, start);
                while let Some(n) = dfs.next(&g) {
                    if !visited[n.index()] {
                        visited[n.index()] = true;
                        data.push((ni_to_id.get(&n).copied().unwrap_or(-1), comp_id));
                    }
                }
                comp_id += 1;
            }
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("component_id", TypeId::Integer);
            Ok(CcState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let comps: Vec<i32> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_i32_col(raw, 1, &comps, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 6. petgraph_pagerank
// ═══════════════════════════════════════════════

struct PrState { data: Vec<(i32, f64)>, offset: usize }

fn build_petgraph_pagerank() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_pagerank")
        .param(TypeId::Varchar).param(TypeId::Double).param(TypeId::Integer)
        .with_state::<PrState, _>(|bind| {
            if bind.parameter_count() < 3 {
                return Err(ExtensionError::new("petgraph_pagerank: expected 3 params (graph, alpha, iterations)"));
            }
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let alpha = unsafe { bind.get_parameter_value(1) }.as_f64_or(0.85);
            let iterations = unsafe { bind.get_parameter_value(2) }.as_i64_or(20) as usize;
            let scores = algo::page_rank(&g, alpha, iterations);
            let mut data: Vec<(i32, f64)> = scores.iter().enumerate()
                .map(|(i, s)| { let ni = NodeIndex::new(i); (ni_to_id.get(&ni).copied().unwrap_or(i as i32), *s) })
                .collect();
            data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("score", TypeId::Double);
            Ok(PrState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_f64_col(raw, 1, &scores, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 7. petgraph_betweenness_centrality
// ═══════════════════════════════════════════════

struct BcState { data: Vec<(i32, f64)>, offset: usize }

fn build_petgraph_betweenness_centrality() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_betweenness_centrality")
        .param(TypeId::Varchar)
        .with_state::<BcState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_undigraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
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
                        if dist[w.index()] < 0 { dist[w.index()] = dist[v.index()] + 1; q.push_back(w); }
                        if dist[w.index()] == dist[v.index()] + 1 { sigma[w.index()] += sigma[v.index()]; pred[w.index()].push(v); }
                    }
                }
                let mut delta = vec![0.0f64; n];
                while let Some(w) = stack.pop() {
                    for &v in &pred[w.index()] {
                        delta[v.index()] += (sigma[v.index()] as f64 / sigma[w.index()] as f64) * (1.0 + delta[w.index()]);
                    }
                    if w != s { cb[w.index()] += delta[w.index()]; }
                }
            }
            let mut data: Vec<(i32, f64)> = cb.iter().enumerate()
                .map(|(i, &s)| { let ni = NodeIndex::new(i); (ni_to_id.get(&ni).copied().unwrap_or(i as i32), s) })
                .collect();
            data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("score", TypeId::Double);
            Ok(BcState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_f64_col(raw, 1, &scores, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 8. petgraph_scc (Tarjan)
// ═══════════════════════════════════════════════

struct SccState { data: Vec<(i32, i32)>, offset: usize }

fn build_petgraph_scc() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_scc")
        .param(TypeId::Varchar)
        .with_state::<SccState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let sccs = algo::tarjan_scc(&g);
            let mut data = Vec::new();
            for (comp_id, scc) in sccs.iter().enumerate() {
                for n in scc { data.push((ni_to_id.get(n).copied().unwrap_or(-1), comp_id as i32)); }
            }
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("scc_id", TypeId::Integer);
            Ok(SccState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let comps: Vec<i32> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_i32_col(raw, 1, &comps, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 9. petgraph_mst (Kruskal)
// ═══════════════════════════════════════════════

struct MstState { data: Vec<(i32, i32, f64)>, offset: usize }

fn build_petgraph_mst() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_mst")
        .param(TypeId::Varchar)
        .with_state::<MstState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_undigraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let data: Vec<(i32, i32, f64)> = algo::min_spanning_tree(&g)
                .filter_map(|el| match el {
                    Element::Edge { source, target, weight } => {
                        let s = NodeIndex::new(source); let t = NodeIndex::new(target);
                        Some((ni_to_id.get(&s).copied().unwrap_or(-1), ni_to_id.get(&t).copied().unwrap_or(-1), weight))
                    }
                    Element::Node { .. } => None,
                }).collect();
            bind.add_result_column("src", TypeId::Integer);
            bind.add_result_column("dst", TypeId::Integer);
            bind.add_result_column("weight", TypeId::Double);
            Ok(MstState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let srcs: Vec<i32> = slice.iter().map(|e| e.0).collect();
            let dsts: Vec<i32> = slice.iter().map(|e| e.1).collect();
            let weights: Vec<f64> = slice.iter().map(|e| e.2).collect();
            unsafe { write_i32_col(raw, 0, &srcs, n) };
            unsafe { write_i32_col(raw, 1, &dsts, n) };
            unsafe { write_f64_col(raw, 2, &weights, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 10. petgraph_closeness_centrality
// ═══════════════════════════════════════════════

fn closeness_centrality(g: &DiGraph<(), f64>) -> Vec<f64> {
    let n = g.node_count();
    let mut closeness = vec![0.0; n];
    if n <= 1 { return closeness; }
    let idx_to_node: Vec<NodeIndex> = g.node_indices().collect();
    let node_to_idx: HashMap<NodeIndex, usize> = idx_to_node.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    for (start_idx, &start) in idx_to_node.iter().enumerate() {
        let mut dist: Vec<Option<usize>> = vec![None; n];
        let mut queue = VecDeque::new();
        dist[start_idx] = Some(0);
        queue.push_back(start);
        while let Some(v) = queue.pop_front() {
            let dv = dist[node_to_idx[&v]].unwrap();
            for w in g.neighbors(v) {
                let wi = node_to_idx[&w];
                if dist[wi].is_none() { dist[wi] = Some(dv + 1); queue.push_back(w); }
            }
        }
        let sum: usize = dist.iter().filter_map(|&d| d).sum();
        let reachable = dist.iter().filter(|&&d| d.is_some()).count();
        if reachable > 1 && sum > 0 {
            closeness[start_idx] = (reachable - 1) as f64 * (reachable - 1) as f64 / ((n - 1) as f64 * sum as f64);
        }
    }
    closeness
}

struct ClState { data: Vec<(i32, f64)>, offset: usize }

fn build_petgraph_closeness_centrality() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_closeness_centrality")
        .param(TypeId::Varchar)
        .with_state::<ClState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let scores = closeness_centrality(&g);
            let mut data: Vec<(i32, f64)> = scores.iter().enumerate()
                .map(|(i, &s)| { let ni = NodeIndex::new(i); (ni_to_id.get(&ni).copied().unwrap_or(i as i32), s) })
                .collect();
            data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("centrality", TypeId::Double);
            Ok(ClState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_f64_col(raw, 1, &scores, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 11. petgraph_eigenvector_centrality
// ═══════════════════════════════════════════════

fn eigenvector_centrality(g: &DiGraph<(), f64>, max_iter: usize, tolerance: f64) -> Vec<f64> {
    let n = g.node_count();
    if n == 0 { return Vec::new(); }
    let idx_to_node: Vec<NodeIndex> = g.node_indices().collect();
    let node_to_idx: HashMap<NodeIndex, usize> = idx_to_node.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut x = vec![1.0; n];
    for _iter in 0..max_iter {
        let mut x_new = vec![0.0; n];
        for (i, node) in idx_to_node.iter().enumerate() {
            for neighbor in g.neighbors(*node) {
                x_new[i] += x[node_to_idx[&neighbor]];
            }
        }
        let norm: f64 = x_new.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm < 1e-15 { break; }
        for v in x_new.iter_mut() { *v /= norm; }
        let diff: f64 = x.iter().zip(x_new.iter()).map(|(a, b)| (a - b).abs()).sum();
        x = x_new;
        if diff < tolerance { break; }
    }
    let max_val = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_val = x.iter().cloned().fold(f64::INFINITY, f64::min);
    if (max_val - min_val).abs() > 1e-10 {
        x.iter_mut().for_each(|v| *v = (*v - min_val) / (max_val - min_val));
    }
    x
}

struct EvState { data: Vec<(i32, f64)>, offset: usize }

fn build_petgraph_eigenvector_centrality() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_eigenvector_centrality")
        .param(TypeId::Varchar).param(TypeId::Integer).param(TypeId::Double)
        .with_state::<EvState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let max_iter = if bind.parameter_count() >= 2 { unsafe { bind.get_parameter_value(1) }.as_i64_or(100) as usize } else { 100 };
            let tolerance = if bind.parameter_count() >= 3 { unsafe { bind.get_parameter_value(2) }.as_f64_or(1e-6) } else { 1e-6 };
            let scores = eigenvector_centrality(&g, max_iter, tolerance);
            let mut data: Vec<(i32, f64)> = scores.iter().enumerate()
                .map(|(i, &s)| { let ni = NodeIndex::new(i); (ni_to_id.get(&ni).copied().unwrap_or(i as i32), s) })
                .collect();
            data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("centrality", TypeId::Double);
            Ok(EvState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let scores: Vec<f64> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_f64_col(raw, 1, &scores, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 12. petgraph_louvain
// ═══════════════════════════════════════════════

fn louvain_community(g: &DiGraph<(), f64>) -> Vec<usize> {
    let n = g.node_count();
    if n == 0 { return Vec::new(); }
    let m = g.edge_count() as f64;
    if m < 1.0 { return (0..n).collect(); }
    let idx_to_node: Vec<NodeIndex> = g.node_indices().collect();
    let node_to_idx: HashMap<NodeIndex, usize> = idx_to_node.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (i, node) in idx_to_node.iter().enumerate() {
        for neighbor in g.neighbors(*node) {
            if let Some(&ni) = node_to_idx.get(&neighbor) { adj[i].push((ni, 1.0)); }
        }
    }
    let mut community: Vec<usize> = (0..n).collect();
    for _pass in 0..20 {
        let mut improved = false;
        let mut comm_total = vec![0.0f64; n];
        for (i, neighbors) in adj.iter().enumerate() {
            for &(_j, w) in neighbors { comm_total[community[i]] += w; }
        }
        for i in 0..n {
            let old_comm = community[i];
            let mut comm_weight: HashMap<usize, f64> = HashMap::new();
            let mut ki = 0.0f64;
            for &(j, w) in &adj[i] { ki += w; *comm_weight.entry(community[j]).or_insert(0.0) += w; }
            if ki == 0.0 { continue; }
            let mut best_comm = old_comm;
            let mut best_delta = 0.0f64;
            let k_i_over_2m = ki / (2.0 * m);
            for (&target_comm, &w_to_comm) in &comm_weight {
                if target_comm == old_comm { continue; }
                let sigma_tot = comm_total[target_comm];
                let delta = (w_to_comm / m) - 2.0 * (sigma_tot / (2.0 * m)) * k_i_over_2m * 2.0;
                if delta > best_delta { best_delta = delta; best_comm = target_comm; }
            }
            if best_comm != old_comm { community[i] = best_comm; improved = true; }
        }
        if !improved { break; }
    }
    let mut comm_map: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0usize;
    let mut result = vec![0usize; n];
    for (i, &comm) in community.iter().enumerate() {
        let compact = *comm_map.entry(comm).or_insert_with(|| { let id = next_id; next_id += 1; id });
        result[i] = compact;
    }
    result
}

struct LvState { data: Vec<(i32, i32)>, offset: usize }

fn build_petgraph_louvain() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_louvain")
        .param(TypeId::Varchar)
        .with_state::<LvState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let communities = louvain_community(&g);
            let mut data: Vec<(i32, i32)> = communities.iter().enumerate()
                .map(|(i, &c)| { let ni = NodeIndex::new(i); (ni_to_id.get(&ni).copied().unwrap_or(i as i32), c as i32) })
                .collect();
            data.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            bind.add_result_column("node", TypeId::Integer);
            bind.add_result_column("community_id", TypeId::Integer);
            Ok(LvState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let comps: Vec<i32> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_i32_col(raw, 1, &comps, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 13. petgraph_toposort
// ═══════════════════════════════════════════════

struct TsState { data: Vec<(i32, i32)>, offset: usize }

fn build_petgraph_toposort() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_toposort")
        .param(TypeId::Varchar)
        .with_state::<TsState, _>(|bind| {
            let (g, _, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let data = match algo::toposort(&g, None) {
                Ok(order) => order.iter().enumerate()
                    .map(|(pos, n)| (pos as i32, ni_to_id.get(n).copied().unwrap_or(-1)))
                    .collect(),
                Err(cycle) => {
                    let node_id = ni_to_id.get(&cycle.node_id()).copied().unwrap_or(-1);
                    return Err(ExtensionError::new(&format!("graph contains a cycle at node {node_id}")));
                }
            };
            bind.add_result_column("position", TypeId::Integer);
            bind.add_result_column("node", TypeId::Integer);
            Ok(TsState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let positions: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let nodes: Vec<i32> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &positions, n) };
            unsafe { write_i32_col(raw, 1, &nodes, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 14. petgraph_is_cyclic
// ═══════════════════════════════════════════════

struct CyState { is_cyclic: bool, done: bool }

fn build_petgraph_is_cyclic() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_is_cyclic")
        .param(TypeId::Varchar)
        .with_state::<CyState, _>(|bind| {
            let (g, _, _) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            bind.add_result_column("is_cyclic", TypeId::Integer);
            Ok(CyState { is_cyclic: algo::is_cyclic_directed(&g), done: false })
        })
        .scan(|state, chunk| {
            if state.done { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            state.done = true;
            let raw = unsafe { chunk.as_raw() };
            unsafe { write_i32_col(raw, 0, &[state.is_cyclic as i32], 1) };
            unsafe { duckdb_data_chunk_set_size(raw, 1) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 15. petgraph_edges
// ═══════════════════════════════════════════════

struct EdState { data: Vec<(i32, i32, f64)>, offset: usize }

fn build_petgraph_edges() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_edges")
        .param(TypeId::Varchar)
        .with_state::<EdState, _>(|bind| {
            let param = unsafe { bind.get_parameter_value(0) }.as_str_or("").to_string();
            let data: Vec<(i32, i32, f64)> = if let Ok((g, _, ni_to_id)) = resolve_digraph(&param) {
                g.edge_references().map(|e| {
                    let (u, v) = g.edge_endpoints(e.id()).unwrap();
                    (ni_to_id.get(&u).copied().unwrap_or(-1), ni_to_id.get(&v).copied().unwrap_or(-1), *e.weight())
                }).collect()
            } else if let Ok((g, _, ni_to_id)) = resolve_undigraph(&param) {
                g.edge_references().map(|e| {
                    let (u, v) = g.edge_endpoints(e.id()).unwrap();
                    (ni_to_id.get(&u).copied().unwrap_or(-1), ni_to_id.get(&v).copied().unwrap_or(-1), *e.weight())
                }).collect()
            } else {
                return Err(ExtensionError::new("petgraph_edges: invalid graph (not inline edges nor @cached)"));
            };
            bind.add_result_column("src", TypeId::Integer);
            bind.add_result_column("dst", TypeId::Integer);
            bind.add_result_column("weight", TypeId::Double);
            Ok(EdState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let srcs: Vec<i32> = slice.iter().map(|e| e.0).collect();
            let dsts: Vec<i32> = slice.iter().map(|e| e.1).collect();
            let weights: Vec<f64> = slice.iter().map(|e| e.2).collect();
            unsafe { write_i32_col(raw, 0, &srcs, n) };
            unsafe { write_i32_col(raw, 1, &dsts, n) };
            unsafe { write_f64_col(raw, 2, &weights, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// 11. petgraph_neighbors
// ═══════════════════════════════════════════════

struct NbState { data: Vec<(i32, f64)>, offset: usize }

fn build_petgraph_neighbors() -> Result<TableFunctionBuilder, ExtensionError> {
    TableFunctionBuilder::new("petgraph_neighbors")
        .param(TypeId::Varchar).param(TypeId::Integer)
        .with_state::<NbState, _>(|bind| {
            if bind.parameter_count() < 2 {
                return Err(ExtensionError::new("petgraph_neighbors: expected 2 params (graph, node)"));
            }
            let (g, id_to_ni, ni_to_id) = resolve_digraph(&unsafe { bind.get_parameter_value(0) }.as_str_or("")).map_err(|e| ExtensionError::new(&e.to_string()))?;
            let node = unsafe { bind.get_parameter_value(1) }.as_i64_or(0) as i32;
            let ni = *id_to_ni.get(&node).ok_or_else(|| ExtensionError::new(&format!("node '{node}' not in graph")))?;
            let mut data: Vec<(i32, f64)> = g.edges(ni).map(|e| {
                let target = g.edge_endpoints(e.id()).unwrap().1;
                (ni_to_id.get(&target).copied().unwrap_or(-1), *e.weight())
            }).collect();
            data.sort_by(|a, b| a.0.cmp(&b.0));
            bind.add_result_column("neighbor", TypeId::Integer);
            bind.add_result_column("weight", TypeId::Double);
            Ok(NbState { data, offset: 0 })
        })
        .scan(|state, chunk| {
            let remaining = state.data.len().saturating_sub(state.offset);
            let n = remaining.min(2048);
            if n == 0 { unsafe { duckdb_data_chunk_set_size(chunk.as_raw(), 0) }; return Ok(()); }
            let raw = unsafe { chunk.as_raw() };
            let slice = &state.data[state.offset..state.offset + n];
            let nodes: Vec<i32> = slice.iter().map(|r| r.0).collect();
            let weights: Vec<f64> = slice.iter().map(|r| r.1).collect();
            unsafe { write_i32_col(raw, 0, &nodes, n) };
            unsafe { write_f64_col(raw, 1, &weights, n) };
            state.offset += n;
            unsafe { duckdb_data_chunk_set_size(raw, n as idx_t) };
            Ok(())
        })
        .build()
}

// ═══════════════════════════════════════════════
// Entrypoint
// ═══════════════════════════════════════════════

use libduckdb_sys::{
    duckdb_extension_access, duckdb_extension_info,
};
use quack_rs::{
    connection::Registrar,
    entry_point::init_extension_v2,
};

fn register_all(reg: &impl Registrar) -> Result<(), ExtensionError> {
    macro_rules! reg {
        ($builder:expr) => { unsafe { reg.register_table($builder?) }?; };
    }
    reg!(build_petgraph_build_graph());
    reg!(build_petgraph_list_graphs());
    reg!(build_petgraph_drop_graph());
    reg!(build_petgraph_shortest_path());
    reg!(build_petgraph_connected_components());
    reg!(build_petgraph_pagerank());
    reg!(build_petgraph_betweenness_centrality());
    reg!(build_petgraph_scc());
    reg!(build_petgraph_mst());
    reg!(build_petgraph_closeness_centrality());
    reg!(build_petgraph_eigenvector_centrality());
    reg!(build_petgraph_louvain());
    reg!(build_petgraph_toposort());
    reg!(build_petgraph_is_cyclic());
    reg!(build_petgraph_edges());
    reg!(build_petgraph_neighbors());
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn petgraph_ext_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    unsafe { init_extension_v2(info, access, quack_rs::DUCKDB_API_VERSION, |con| register_all(con)) }
}

// ═══════════════════════════════════════════════
// WASM re-export
// ═══════════════════════════════════════════════

// Exported by wasm_lib.rs
