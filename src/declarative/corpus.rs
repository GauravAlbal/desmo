//! Editable starter corpora. Examples are type-valid inputs, not a domain.

use super::def::{CommandDef, Definition};
use super::explore::CaseCorpus;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const STARTER_NOTE: &str = "example cases, not an exhaustive domain";

/// Load a `CaseCorpus` from an explicit path, filling zero-argument commands
/// with a `"default"` empty-arg case when omitted.
pub fn load_corpus_from_path(path: &Path, def: &Definition) -> Result<CaseCorpus, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut corpus: CaseCorpus =
        serde_json::from_str(&text).map_err(|e| format!("corpus {}: {e}", path.display()))?;
    fill_zero_arg_defaults(&mut corpus, def);
    Ok(corpus)
}

/// Resolve an explicit corpus file or sibling `<machine>.cases.json`, falling back
/// to an empty corpus seeded only with zero-arg command defaults.
pub fn load_corpus(
    machine: &Path,
    explicit: Option<&Path>,
    def: &Definition,
) -> Result<CaseCorpus, String> {
    let sibling = machine.with_extension("cases.json");
    let path = explicit.or_else(|| sibling.exists().then_some(sibling.as_path()));
    match path {
        Some(path) => load_corpus_from_path(path, def),
        None => {
            let mut corpus = CaseCorpus::new();
            fill_zero_arg_defaults(&mut corpus, def);
            Ok(corpus)
        }
    }
}

fn fill_zero_arg_defaults(corpus: &mut CaseCorpus, def: &Definition) {
    for (name, cmd) in &def.commands {
        if cmd.args.as_ref().map(|a| a.is_empty()).unwrap_or(true)
            && !corpus.commands.contains_key(name)
        {
            corpus.insert(name.clone(), "default", BTreeMap::new());
        }
    }
}

/// One type-valid example per command, plus one min and one max case for each
/// command that declares an `int` argument. `int` is the only bounded argument
/// type in schema 1; its ends are the shared signed 64-bit range.
pub fn starter_corpus(def: &Definition) -> Value {
    let mut commands = Map::new();
    for (name, cmd) in &def.commands {
        commands.insert(name.clone(), Value::Array(cases_for(cmd)));
    }
    json!({
        "_note": STARTER_NOTE,
        "commands": commands,
    })
}

fn cases_for(cmd: &CommandDef) -> Vec<Value> {
    let declared = cmd.args.clone().unwrap_or_default();
    let mut cases = vec![case("example", &declared, None)];
    if declared.values().any(|ty| ty == "int") {
        cases.push(case("boundary_min", &declared, Some(i64::MIN)));
        cases.push(case("boundary_max", &declared, Some(i64::MAX)));
    }
    cases
}

fn case(name: &str, declared: &BTreeMap<String, String>, bound: Option<i64>) -> Value {
    let mut args = Map::new();
    for (arg, ty) in declared {
        args.insert(arg.clone(), arg_value(ty, bound));
    }
    json!({"name": name, "args": args})
}

fn arg_value(ty: &str, bound: Option<i64>) -> Value {
    if ty == "int" {
        return json!(bound.unwrap_or(1));
    }
    match ty {
        "bool" => json!(true),
        "string" => json!("example"),
        _ => Value::Null,
    }
}
