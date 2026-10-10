# duckdb-petgraph — Claude Code Plugin

One skill giving your AI assistant verified knowledge of the
[duckdb_petgraph](https://github.com/alitrack/duckdb_petgraph) extension:
16 graph algorithms, three ways to build graphs (inline, cached, LIST-typed
from tables), and canonical patterns like dependency cycle hunting.

## Install

```text
/plugin marketplace add alitrack/duckdb_petgraph
/plugin install duckdb-petgraph@duckdb-petgraph
```

## Skills

| Skill | Covers |
|---|---|
| `duckdb-petgraph-algos` | algorithm picker, build_graph/build_graph_list, SET VARIABLE bridge |

```sql
INSTALL petgraph_ext FROM community; LOAD petgraph_ext;
```

MIT, alitrack.
