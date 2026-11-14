//! `ik diagram` CLI.

use super::{build_with, machines_in_evidence, DiagramFormat, DiagramKind};
use crate::declarative::load_definition;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const USAGE: &str = "\
usage: ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json] [--focus <state>] [--level 0|1|2] [--ascii] [--operation <id>]
";

struct Invocation {
    machine: PathBuf,
    prior: Option<PathBuf>,
    evidence: Vec<PathBuf>,
    kind: DiagramKind,
    format: DiagramFormat,
    focus: Option<String>,
    level: u8,
    ascii: bool,
    operation: Option<String>,
}

enum Flag {
    Help,
    Prior(PathBuf),
    Evidence(PathBuf),
    Kind(DiagramKind),
    Format(DiagramFormat),
    Focus(String),
    Level(u8),
    Ascii,
    Operation(String),
    Positional(PathBuf),
}

fn take_flag(args: &[String], index: &mut usize) -> Result<Flag, String> {
    let token = args[*index].clone();
    *index += 1;
    if let Some(flag) = bare_flag(&token) {
        return Ok(flag);
    }
    if token.starts_with('-') {
        return valued_flag(&token, args, index);
    }
    Ok(Flag::Positional(PathBuf::from(token)))
}

fn bare_flag(token: &str) -> Option<Flag> {
    match token {
        "-h" | "--help" => Some(Flag::Help),
        "--ascii" => Some(Flag::Ascii),
        _ => None,
    }
}

enum ValueKind {
    Operation,
    Prior,
    Evidence,
    Kind,
    Format,
    Focus,
    Level,
}

fn valued_flag(token: &str, args: &[String], index: &mut usize) -> Result<Flag, String> {
    let (kind, missing) = value_kind(token)?;
    let raw = need_str(args, index, missing)?;
    flag_from_value(kind, raw)
}

fn value_kind(token: &str) -> Result<(ValueKind, &'static str), String> {
    match token {
        "--operation" => Ok((ValueKind::Operation, "--operation requires an id")),
        "--prior" => Ok((ValueKind::Prior, "--prior requires a path")),
        "--evidence" => Ok((ValueKind::Evidence, "--evidence requires a path")),
        "--kind" => Ok((ValueKind::Kind, "--kind requires a value")),
        "--format" => Ok((ValueKind::Format, "--format requires a value")),
        "--focus" => Ok((ValueKind::Focus, "--focus requires a state")),
        "--level" => Ok((ValueKind::Level, "--level requires 0, 1, or 2")),
        flag => Err(format!("unknown flag `{flag}`")),
    }
}

fn flag_from_value(kind: ValueKind, raw: String) -> Result<Flag, String> {
    match kind {
        ValueKind::Operation => Ok(Flag::Operation(raw)),
        ValueKind::Prior => Ok(Flag::Prior(PathBuf::from(raw))),
        ValueKind::Evidence => Ok(Flag::Evidence(PathBuf::from(raw))),
        ValueKind::Focus => Ok(Flag::Focus(raw)),
        ValueKind::Kind => Ok(Flag::Kind(DiagramKind::parse(&raw)?)),
        ValueKind::Format => Ok(Flag::Format(DiagramFormat::parse(&raw)?)),
        ValueKind::Level => Ok(Flag::Level(parse_level(&raw)?)),
    }
}

fn need_str(args: &[String], index: &mut usize, error: &str) -> Result<String, String> {
    let value = args.get(*index).ok_or_else(|| error.to_string())?.clone();
    *index += 1;
    Ok(value)
}

fn parse_level(raw: &str) -> Result<u8, String> {
    match raw {
        "0" => Ok(0),
        "1" => Ok(1),
        "2" => Ok(2),
        other => Err(format!("--level must be 0, 1, or 2 (got `{other}`)")),
    }
}

pub fn run(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(Some(invocation)) => match draw(&invocation) {
            Ok(body) => {
                print!("{body}");
                0
            }
            Err(message) if message.starts_with("ambiguous:") => {
                println!("{message}");
                2
            }
            Err(message) => usage_error(&message),
        },
        Ok(None) => {
            print!("{USAGE}");
            0
        }
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik diagram: {message}");
    eprint!("{USAGE}");
    2
}

fn parse_args(args: &[String]) -> Result<Option<Invocation>, String> {
    let mut acc = Acc::default();
    let mut index = 0;
    while index < args.len() {
        if apply_flag(&mut acc, take_flag(args, &mut index)?)? {
            return Ok(None);
        }
    }
    acc.finish()
}

struct Acc {
    machine: Option<PathBuf>,
    prior: Option<PathBuf>,
    evidence: Vec<PathBuf>,
    kind: DiagramKind,
    format: DiagramFormat,
    focus: Option<String>,
    level: u8,
    ascii: bool,
    operation: Option<String>,
}

impl Default for Acc {
    fn default() -> Self {
        Self {
            machine: None,
            prior: None,
            evidence: Vec::new(),
            kind: DiagramKind::Delivery,
            format: DiagramFormat::Markdown,
            focus: None,
            level: 1,
            ascii: false,
            operation: None,
        }
    }
}

impl Acc {
    fn finish(self) -> Result<Option<Invocation>, String> {
        Ok(Some(Invocation {
            machine: self.machine.ok_or("diagram requires <machine.ik.toml>")?,
            prior: self.prior,
            evidence: self.evidence,
            kind: self.kind,
            format: self.format,
            focus: self.focus,
            level: self.level,
            ascii: self.ascii,
            operation: self.operation,
        }))
    }
}

fn apply_flag(acc: &mut Acc, flag: Flag) -> Result<bool, String> {
    match flag {
        Flag::Help => return Ok(true),
        Flag::Prior(path) => acc.prior = Some(path),
        Flag::Evidence(path) => acc.evidence.push(path),
        Flag::Kind(value) => acc.kind = value,
        Flag::Format(value) => acc.format = value,
        Flag::Focus(value) => acc.focus = Some(value),
        Flag::Level(value) => acc.level = value,
        Flag::Ascii => acc.ascii = true,
        Flag::Operation(value) => acc.operation = Some(value),
        Flag::Positional(path) => set_machine(acc, path)?,
    }
    Ok(false)
}

fn set_machine(acc: &mut Acc, path: PathBuf) -> Result<(), String> {
    if acc.machine.is_some() {
        return Err(format!("unexpected argument `{}`", path.display()));
    }
    acc.machine = Some(path);
    Ok(())
}

fn draw(invocation: &Invocation) -> Result<String, String> {
    let def = Some(load_definition(&invocation.machine).map_err(|errors| errors.join("; "))?);
    let prior = match &invocation.prior {
        Some(path) => Some(load_definition(path).map_err(|errors| errors.join("; "))?),
        None => {
            let (_, discovered) = machines_in_evidence(&invocation.evidence);
            match discovered {
                Some(path) => Some(load_definition(&path).map_err(|errors| errors.join("; "))?),
                None => None,
            }
        }
    };
    let findings = load_findings(&invocation.evidence);
    let subject = def
        .as_ref()
        .map(|item| item.machine_id.as_str())
        .unwrap_or("");
    let named = crate::diagram::distinct_operation_ids(&findings);
    let operation = match &invocation.operation {
        Some(id) => Some(id.clone()),
        None if named.len() > 1 => {
            return Err(format!(
                "ambiguous: evidence names {} operations ({}); select one with --operation <id>",
                named.len(),
                named.join(", ")
            ));
        }
        None if named.len() == 1 => Some(named[0].clone()),
        None => None,
    };
    let focus = invocation.focus.as_deref();
    let diagram = build_with(
        def.as_ref(),
        prior.as_ref(),
        &findings,
        invocation.kind,
        subject,
        operation.as_deref(),
        invocation.level,
        focus,
        invocation.ascii,
        Some(invocation.machine.as_path()),
    );
    let body = match invocation.format {
        DiagramFormat::Markdown => diagram.to_markdown(),
        DiagramFormat::Text => diagram.to_compressed_view(),
        DiagramFormat::Mermaid => diagram.to_mermaid(),
        DiagramFormat::Table => diagram.to_table_markdown(),
        DiagramFormat::Json => crate::canonical::canonical_json(&diagram.json_field()),
    };
    Ok(body)
}

fn load_findings(paths: &[PathBuf]) -> Vec<Value> {
    if paths.is_empty() {
        return Vec::new();
    }
    let mut direct = Vec::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(arr) = value.get("findings").and_then(Value::as_array) {
            direct.extend(arr.clone());
        } else if value.get("status").is_some() {
            direct.push(value);
        }
    }
    if !direct.is_empty() {
        return direct;
    }
    let report = crate::report::report_value(paths);
    report["findings"].as_array().cloned().unwrap_or_default()
}
