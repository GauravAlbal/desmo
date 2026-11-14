//! Editable starter corpora. Examples are type-valid inputs, not a domain.

use super::def::{CommandDef, Definition};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const STARTER_NOTE: &str = "example cases, not an exhaustive domain";

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
