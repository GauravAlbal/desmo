//! `ik graph export`: a finite graph from the existing explorer.
//!
//! The document is `ik.graph.v1`. It records the states and command outcomes the
//! authoritative reducer produced under the supplied corpus and bounds. It does
//! not explore a second time and it does not drop an edge that left the domain.

use crate::canonical::canonical_json;
use crate::declarative::explore::{explore_graph, Bounds, CaseCorpus, Exploration, RuntimeGraph};
use crate::declarative::load_def;
use crate::declarative::profile::{DEFAULT_MAX_DEPTH, DEFAULT_MAX_NODES};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage: ik graph export <machine.ik.toml> [--corpus cases.json] [--max-nodes N] [--max-depth D] --out graph.json
";

pub const GRAPH_SCHEMA: &str = "ik.graph.v1";

struct ExportArgs {
    machine: PathBuf,
    corpus: Option<PathBuf>,
    out: PathBuf,
    bounds: Bounds,
}

#[derive(Serialize)]
struct GraphDocument<'a> {
    schema: &'static str,
    machine_id: &'a str,
    spec_hash: &'a str,
    corpus_digest: &'a str,
    bounds: Bounds,
    complete: bool,
    incomplete_reason: Option<&'a str>,
    initial: &'a str,
    nodes: &'a [crate::declarative::explore::GraphNode],
    edges: &'a [crate::declarative::explore::GraphEdge],
    frontier: &'a [String],
    restrictions: &'a [crate::declarative::explore::GraphRestriction],
}

/// CLI entry for `ik graph`. Exit 0 writes the graph, 2 is usage or load failure.
pub fn run(args: &[String]) -> i32 {
    match dispatch(args) {
        Ok(code) => code,
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik graph: {message}");
    eprintln!("{USAGE}");
    2
}

fn dispatch(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return Ok(0);
    }
    let parsed = parse_export(args)?;
    let machine = load_def(&parsed.machine).map_err(|errors| errors.join("; "))?;
    let corpus = load_corpus(parsed.corpus.as_deref())?;
    let (exploration, graph) = explore_graph(&machine, &corpus, parsed.bounds);
    write_graph(&parsed.out, &exploration, &graph)
}

fn parse_export(args: &[String]) -> Result<ExportArgs, String> {
    if args.first().map(String::as_str) != Some("export") {
        return Err("graph command is required".into());
    }
    let mut machine = None;
    let mut corpus = None;
    let mut out = None;
    let mut max_nodes = DEFAULT_MAX_NODES;
    let mut max_depth = DEFAULT_MAX_DEPTH;
    let mut index = 1;
    while index < args.len() {
        index = take_export_arg(
            args,
            index,
            &mut machine,
            &mut corpus,
            &mut out,
            &mut max_nodes,
            &mut max_depth,
        )?;
    }
    finish_export(machine, corpus, out, max_nodes, max_depth)
}

fn take_export_arg(
    args: &[String],
    index: usize,
    machine: &mut Option<PathBuf>,
    corpus: &mut Option<PathBuf>,
    out: &mut Option<PathBuf>,
    max_nodes: &mut usize,
    max_depth: &mut usize,
) -> Result<usize, String> {
    match args[index].as_str() {
        "--corpus" => assign_path(args, index, corpus),
        "--out" => assign_path(args, index, out),
        "--max-nodes" => assign_bound(args, index, max_nodes),
        "--max-depth" => assign_bound(args, index, max_depth),
        flag if flag.starts_with('-') => Err(format!("unknown flag `{flag}`")),
        path => {
            one_machine(machine, path)?;
            Ok(index + 1)
        }
    }
}

fn assign_path(args: &[String], index: usize, slot: &mut Option<PathBuf>) -> Result<usize, String> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{} requires a path", args[index]))?;
    *slot = Some(PathBuf::from(value));
    Ok(index + 2)
}

fn assign_bound(args: &[String], index: usize, slot: &mut usize) -> Result<usize, String> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{} requires a number", args[index]))?;
    *slot = value
        .parse()
        .map_err(|_| format!("invalid bound `{value}`"))?;
    Ok(index + 2)
}

fn one_machine(slot: &mut Option<PathBuf>, path: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("unexpected argument `{path}`"));
    }
    *slot = Some(PathBuf::from(path));
    Ok(())
}

fn finish_export(
    machine: Option<PathBuf>,
    corpus: Option<PathBuf>,
    out: Option<PathBuf>,
    max_nodes: usize,
    max_depth: usize,
) -> Result<ExportArgs, String> {
    let machine = machine.ok_or_else(|| "export requires <machine.ik.toml>".to_string())?;
    let out = out.ok_or_else(|| "--out <graph.json> is required".to_string())?;
    Ok(ExportArgs {
        machine,
        corpus,
        out,
        bounds: Bounds {
            max_nodes,
            max_depth,
        },
    })
}

fn load_corpus(path: Option<&Path>) -> Result<CaseCorpus, String> {
    let Some(path) = path else {
        return Ok(CaseCorpus::new());
    };
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("corpus {}: {err}", path.display()))
}

fn write_graph(
    path: &Path,
    exploration: &Exploration,
    graph: &RuntimeGraph,
) -> Result<i32, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
    }
    let document = document_of(exploration, graph);
    let value = serde_json::to_value(&document).map_err(|err| err.to_string())?;
    fs::write(path, canonical_json(&value))
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(0)
}

fn document_of<'a>(exploration: &'a Exploration, graph: &'a RuntimeGraph) -> GraphDocument<'a> {
    GraphDocument {
        schema: GRAPH_SCHEMA,
        machine_id: &exploration.machine_id,
        spec_hash: &exploration.spec_hash,
        corpus_digest: &exploration.case_corpus_digest,
        bounds: exploration.bound,
        complete: exploration.complete,
        incomplete_reason: exploration.incomplete_reason.as_deref(),
        initial: &graph.initial,
        nodes: &graph.nodes,
        edges: &graph.edges,
        frontier: &graph.frontier,
        restrictions: &graph.restrictions,
    }
}
