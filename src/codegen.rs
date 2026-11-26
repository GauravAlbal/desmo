//! `ik codegen`: unified deterministic typed client bindings for TypeScript, Python, and Go
//! directly from a compiled `.ik.toml` machine definition.
//!
//! Generated bindings are disposable host-language projections, never runtime authority:
//! the Rust `ik` kernel enforces every guard, invariant, and capability check at runtime.

use crate::declarative::{load_def, Definition};
use crate::identity::MachineIdentity;
use crate::machine::Machine;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik codegen <machine.ik.toml> --lang ts|py|go [--out <file>] [--check] [--import-from <specifier>] [--package <name>]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetLang {
    TypeScript,
    Python,
    Go,
}

impl TargetLang {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "ts" | "typescript" => Ok(Self::TypeScript),
            "py" | "python" => Ok(Self::Python),
            "go" | "golang" => Ok(Self::Go),
            other => Err(format!(
                "unsupported --lang `{other}` (expected one of: ts, py, go)"
            )),
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik codegen: {msg}");
            eprintln!("{USAGE}");
            2
        }
    }
}

fn run_inner(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return Ok(0);
    }
    let mut machine: Option<PathBuf> = None;
    let mut lang: Option<TargetLang> = None;
    let mut out: Option<PathBuf> = None;
    let mut check = false;
    let mut import_from: Option<String> = None;
    let mut go_package: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" => {
                i += 1;
                let value = args.get(i).ok_or("--lang requires ts|py|go")?;
                lang = Some(TargetLang::parse(value)?);
            }
            "--out" => {
                i += 1;
                let value = args.get(i).ok_or("--out requires a path")?;
                out = Some(PathBuf::from(value));
            }
            "--check" => check = true,
            "--import-from" => {
                i += 1;
                let value = args.get(i).ok_or("--import-from requires a specifier")?;
                import_from = Some(value.clone());
            }
            "--package" => {
                i += 1;
                let value = args.get(i).ok_or("--package requires a Go package name")?;
                go_package = Some(value.clone());
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path if machine.is_none() => machine = Some(PathBuf::from(path)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    let machine = machine.ok_or("machine definition path is required")?;
    let lang = lang.ok_or("--lang ts|py|go is required")?;
    let compiled = load_def(&machine).map_err(|errors| errors.join("; "))?;
    let generated = generate(
        compiled.definition(),
        compiled.identity(),
        lang,
        import_from.as_deref(),
        go_package.as_deref(),
    );

    if check {
        let out_path = out.ok_or("--check requires --out <file>")?;
        match std::fs::read_to_string(&out_path) {
            Ok(existing) if existing == generated => return Ok(0),
            Ok(_) => {
                eprintln!(
                    "ik codegen: {} is stale; regenerate with `ik codegen {} --lang ... --out {}`",
                    out_path.display(),
                    machine.display(),
                    out_path.display()
                );
                return Ok(1);
            }
            Err(_) => {
                eprintln!("ik codegen: {} is missing", out_path.display());
                return Ok(1);
            }
        }
    }

    if let Some(out_path) = out {
        write_file(&out_path, &generated)?;
        println!(
            "wrote {} ({} v{}, {:?})",
            out_path.display(),
            compiled.identity().machine_id,
            compiled.identity().version,
            lang
        );
    } else {
        print!("{generated}");
    }
    Ok(0)
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create directory {}: {err}", parent.display()))?;
        }
    }
    std::fs::write(path, content).map_err(|err| format!("write {}: {err}", path.display()))
}

/// Pure, deterministic code generation for a compiled declarative machine.
pub fn generate(
    def: &Definition,
    identity: &MachineIdentity,
    lang: TargetLang,
    import_from: Option<&str>,
    go_package: Option<&str>,
) -> String {
    match lang {
        TargetLang::TypeScript => generate_ts(def, identity, import_from.unwrap_or("@desmo/runtime")),
        TargetLang::Python => generate_py(def, identity, import_from.unwrap_or("invariant_kernel")),
        TargetLang::Go => generate_go(
            def,
            identity,
            import_from.unwrap_or("github.com/GauravAlbal/desmo/go"),
            go_package,
        ),
    }
}

fn pascal_case(raw: &str) -> String {
    let mut out = String::new();
    for part in raw.split(|c: char| !c.is_alphanumeric()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    if out.is_empty() {
        "Machine".to_string()
    } else {
        out
    }
}

fn snake_case(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

fn go_pkg_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("m{s}")
    } else {
        s
    }
}

fn command_successors(cmd: &crate::declarative::def::CommandDef) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(outcomes) = &cmd.outcomes {
        for o in outcomes {
            if !out.contains(&o.to) {
                out.push(o.to.clone());
            }
        }
    } else if let Some(to) = &cmd.to {
        out.push(to.clone());
    }
    out
}

fn generate_ts(def: &Definition, identity: &MachineIdentity, import_from: &str) -> String {
    let prefix = pascal_case(&identity.machine_id);
    let states: Vec<&String> = def.states.keys().collect();
    let terminals: Vec<&String> = def
        .states
        .iter()
        .filter(|(_, s)| s.terminal)
        .map(|(k, _)| k)
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "// GENERATED by `ik codegen --lang ts` from machine {} v{}. Do not edit.\n\
         // Disposable host-language projection: the Rust `ik` kernel enforces all rules at runtime.\n\
         import {{ HistoricalResult, type Aggregate, type Json, type Machine }} from {:?};\n\n\
         export const MACHINE = {{\n  machine_id: {:?},\n  version: {:?},\n  spec_hash: {:?},\n}} as const;\n\n",
        identity.machine_id,
        identity.version,
        import_from,
        identity.machine_id,
        identity.version.0.to_string(),
        identity.spec_hash.0,
    ));

    let state_lits: Vec<String> = states.iter().map(|s| format!("{s:?}")).collect();
    let term_union = if terminals.is_empty() {
        "never".to_string()
    } else {
        terminals
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    out.push_str(&format!(
        "export const STATES = [{}] as const;\n\
         export type StateName = (typeof STATES)[number];\n\
         export type TerminalState = {term_union};\n\n\
         export interface Facts {{\n",
        state_lits.join(", ")
    ));
    for (name, fact) in &def.facts {
        let ts_ty = match fact.ty.as_str() {
            "int" => "number | bigint",
            "bool" => "boolean",
            _ => "string",
        };
        let opt = if fact.initial.is_some() { "" } else { "?" };
        out.push_str(&format!("  readonly {name:?}{opt}: {ts_ty};\n"));
    }
    out.push_str("}\n\nexport type CommandArgs = {\n");
    for (name, cmd) in &def.commands {
        let args = cmd.args.as_ref();
        if args.is_none_or(|m| m.is_empty()) {
            out.push_str(&format!("  {name:?}: Record<string, never>;\n"));
        } else {
            let fields: Vec<String> = args
                .unwrap()
                .iter()
                .map(|(k, ty)| {
                    let ts_ty = match ty.as_str() {
                        "int" => "number | bigint",
                        "bool" => "boolean",
                        _ => "string",
                    };
                    format!("{k:?}: {ts_ty}")
                })
                .collect();
            out.push_str(&format!("  {name:?}: {{ {} }};\n", fields.join("; ")));
        }
    }
    out.push_str("};\n\nexport type Command =\n");
    if def.commands.is_empty() {
        out.push_str("  never;\n");
    } else {
        for (name, cmd) in &def.commands {
            if cmd.args.as_ref().is_none_or(|m| m.is_empty()) {
                out.push_str(&format!("  | {{ name: {name:?}; args?: never }}\n"));
            } else {
                out.push_str(&format!("  | {{ name: {name:?}; args: CommandArgs[{name:?}] }}\n"));
            }
        }
        out.push_str(";\n");
    }
    out.push_str("export type CommandName = Command[\"name\"];\n\n");

    out.push_str(
        "export class StaleContractError extends Error {\n\
           constructor(message: string) {\n\
             super(message);\n\
             this.name = \"StaleContractError\";\n\
           }\n\
         }\n\n\
         export function assertMachine(machine: {\n\
           identity: { machine_id: string; version: number | bigint | string; spec_hash: string };\n\
         }): void {\n\
           const { identity } = machine;\n\
           if (\n\
             identity.machine_id !== MACHINE.machine_id ||\n\
             String(identity.version) !== MACHINE.version ||\n\
             identity.spec_hash !== MACHINE.spec_hash\n\
           ) {\n\
             throw new StaleContractError(\n\
               `generated contract is for ${MACHINE.machine_id} v${MACHINE.version} (${MACHINE.spec_hash}); ` +\n\
                 `bound machine is ${identity.machine_id} v${String(identity.version)} (${identity.spec_hash}).`,\n\
             );\n\
           }\n\
         }\n\n",
    );

    out.push_str(&format!(
        "export class {prefix}Aggregate {{\n\
           readonly raw: Aggregate;\n\
           constructor(raw: Aggregate) {{\n\
             assertMachine(raw.machine);\n\
             this.raw = raw;\n\
           }}\n\
           get id(): string {{ return this.raw.id; }}\n\
           get generation(): number | bigint {{ return this.raw.generation; }}\n\
           get terminal(): boolean {{ return this.raw.terminal; }}\n\
           get stateName(): StateName {{ return this.raw.namedState() as StateName; }}\n\
           facts(): Facts {{\n\
             const obj = (this.raw.state && typeof this.raw.state === \"object\" && !Array.isArray(this.raw.state))\n\
               ? (this.raw.state as Record<string, unknown>)\n\
               : {{}};\n\
             const {{ state: _ignored, ...rest }} = obj;\n\
             return rest as unknown as Facts;\n\
           }}\n",
    ));
    for (name, cmd) in &def.commands {
        let method = snake_case(name);
        let has_args = cmd.args.as_ref().is_some_and(|m| !m.is_empty());
        if has_args {
            out.push_str(&format!(
                "  async {method}(args: CommandArgs[{name:?}], options?: {{ operationId?: string }}): Promise<{prefix}Aggregate | HistoricalResult> {{\n\
                   const res = await this.raw.execute({{ name: {name:?}, args: args as unknown as Record<string, Json> }}, options);\n\
                   return res instanceof HistoricalResult ? res : new {prefix}Aggregate(res);\n\
                 }}\n"
            ));
        } else {
            out.push_str(&format!(
                "  async {method}(options?: {{ operationId?: string }}): Promise<{prefix}Aggregate | HistoricalResult> {{\n\
                   const res = await this.raw.execute({{ name: {name:?} }}, options);\n\
                   return res instanceof HistoricalResult ? res : new {prefix}Aggregate(res);\n\
                 }}\n"
            ));
        }
    }
    out.push_str(&format!(
        "}}\n\n\
         export class {prefix}Machine {{\n\
           readonly raw: Machine;\n\
           constructor(raw: Machine) {{\n\
             assertMachine(raw);\n\
             this.raw = raw;\n\
           }}\n\
           async create(aggregateId: string): Promise<{prefix}Aggregate> {{\n\
             return new {prefix}Aggregate(await this.raw.create(aggregateId));\n\
           }}\n\
           async show(aggregateId: string): Promise<{prefix}Aggregate> {{\n\
             return new {prefix}Aggregate(await this.raw.show(aggregateId));\n\
           }}\n\
         }}\n"
    ));
    out
}

fn generate_py(def: &Definition, identity: &MachineIdentity, import_from: &str) -> String {
    let prefix = pascal_case(&identity.machine_id);
    let states: Vec<String> = def.states.keys().map(|s| format!("{s:?}")).collect();
    let mut out = String::new();
    out.push_str(&format!(
        "# GENERATED by `ik codegen --lang py` from machine {} v{}. Do not edit.\n\
         # Disposable host-language projection: the Rust `ik` kernel enforces all rules at runtime.\n\
         from __future__ import annotations\n\n\
         from typing import Any, Literal, Optional, TypedDict\n\
         from {import_from} import Aggregate, HistoricalResult, Machine\n\n\
         MACHINE_ID = {:?}\n\
         MACHINE_VERSION = {}\n\
         SPEC_HASH = {:?}\n\n\
         StateName = Literal[{}]\n\n\
         class Facts(TypedDict, total=False):\n",
        identity.machine_id,
        identity.version,
        identity.machine_id,
        identity.version.0,
        identity.spec_hash.0,
        states.join(", "),
    ));
    if def.facts.is_empty() {
        out.push_str("    pass\n");
    } else {
        for (name, fact) in &def.facts {
            let py_ty = match fact.ty.as_str() {
                "int" => "int",
                "bool" => "bool",
                _ => "str",
            };
            out.push_str(&format!("    {name}: {py_ty}\n"));
        }
    }
    out.push_str(&format!(
        "\n\nclass StaleContractError(RuntimeError):\n    pass\n\n\n\
         def assert_machine(machine: Machine) -> None:\n\
             ident = machine.identity\n\
             wire_hash = ident.to_wire().get(\"spec_hash\")\n\
             if (\n\
                 ident.machine_id != MACHINE_ID\n\
                 or ident.version != MACHINE_VERSION\n\
                 or wire_hash != SPEC_HASH\n\
             ):\n\
                 raise StaleContractError(\n\
                     f\"generated contract is for {{MACHINE_ID}} v{{MACHINE_VERSION}} ({{SPEC_HASH}}); \"\n\
                     f\"bound machine is {{ident.machine_id}} v{{ident.version}} ({{wire_hash}})\"\n\
                 )\n\n\n\
         class {prefix}Aggregate:\n\
             def __init__(self, raw: Aggregate) -> None:\n\
                 assert_machine(raw._machine)\n\
                 self.raw = raw\n\n\
             @property\n\
             def id(self) -> str:\n\
                 return self.raw.id\n\n\
             @property\n\
             def generation(self) -> int:\n\
                 return self.raw.generation\n\n\
             @property\n\
             def terminal(self) -> bool:\n\
                 return self.raw.terminal\n\n\
             @property\n\
             def state_name(self) -> StateName:\n\
                 return self.raw.state  # type: ignore[return-value]\n\n\
             @property\n\
             def facts(self) -> Facts:\n\
                 return self.raw.facts  # type: ignore[return-value]\n"
    ));

    for (name, cmd) in &def.commands {
        let method = snake_case(name);
        let args = cmd.args.as_ref();
        let mut params = vec!["self".to_string()];
        let mut arg_dict_entries = Vec::new();
        if let Some(map) = args {
            if !map.is_empty() {
                params.push("*".to_string());
                for (k, ty) in map {
                    let py_ty = match ty.as_str() {
                        "int" => "int",
                        "bool" => "bool",
                        _ => "str",
                    };
                    params.push(format!("{k}: {py_ty}"));
                    arg_dict_entries.push(format!("{k:?}: {k}"));
                }
                params.push("operation_id: Optional[str] = None".to_string());
            } else {
                params.push("*, operation_id: Optional[str] = None".to_string());
            }
        } else {
            params.push("*, operation_id: Optional[str] = None".to_string());
        }
        let body_args = if arg_dict_entries.is_empty() {
            "{}".to_string()
        } else {
            format!("{{{}}}", arg_dict_entries.join(", "))
        };
        out.push_str(&format!(
            "\n    def {method}({}) -> {prefix}Aggregate | HistoricalResult:\n\
                 res = self.raw.execute({{\"name\": {name:?}, \"args\": {body_args}}}, operation_id=operation_id)\n\
                 if isinstance(res, HistoricalResult):\n\
                     return res\n\
                 return {prefix}Aggregate(res)\n",
            params.join(", ")
        ));
    }

    out.push_str(&format!(
        "\n\nclass {prefix}Machine:\n\
             def __init__(self, raw: Machine) -> None:\n\
                 assert_machine(raw)\n\
                 self.raw = raw\n\n\
             def create(self, aggregate_id: str) -> {prefix}Aggregate:\n\
                 return {prefix}Aggregate(self.raw.create(aggregate_id))\n\n\
             def show(self, aggregate_id: str) -> {prefix}Aggregate:\n\
                 return {prefix}Aggregate(self.raw.show(aggregate_id))\n"
    ));
    let _ = command_successors;
    out
}

fn generate_go(
    def: &Definition,
    identity: &MachineIdentity,
    import_from: &str,
    go_package: Option<&str>,
) -> String {
    let pkg = go_package
        .map(str::to_string)
        .unwrap_or_else(|| go_pkg_name(&identity.machine_id));
    let prefix = pascal_case(&identity.machine_id);
    let mut out = String::new();
    out.push_str(&format!(
        "// GENERATED by `ik codegen --lang go` from machine {} v{}. Do not edit.\n\
         // Disposable host-language projection: the Rust `ik` kernel enforces all rules at runtime.\n\
         package {pkg}\n\n\
         import (\n\
         \t\"context\"\n\
         \t\"fmt\"\n\n\
         \tik {import_from:?}\n\
         )\n\n\
         const (\n\
         \tMachineID      = {:?}\n\
         \tMachineVersion = uint64({})\n\
         \tSpecHash       = {:?}\n\
         )\n\n\
         type StateName string\n\n\
         const (\n",
        identity.machine_id,
        identity.version,
        identity.machine_id,
        identity.version.0,
        identity.spec_hash.0,
    ));
    for state in def.states.keys() {
        out.push_str(&format!(
            "\tState{} StateName = {:?}\n",
            pascal_case(state),
            state
        ));
    }
    out.push_str(")\n\n");
    out.push_str(&format!(
        "func AssertMachine(m *ik.Machine) error {{\n\
         \tif m.Identity.MachineID != MachineID || m.Identity.Version != MachineVersion || m.Identity.SpecHash() != SpecHash {{\n\
         \t\treturn fmt.Errorf(\"generated contract is for %s v%d (%s); bound machine is %s v%d (%s)\",\n\
         \t\t\tMachineID, MachineVersion, SpecHash, m.Identity.MachineID, m.Identity.Version, m.Identity.SpecHash())\n\
         \t}}\n\
         \treturn nil\n\
         }}\n\n\
         type {prefix}Machine struct {{ Raw *ik.Machine }}\n\n\
         func Wrap{prefix}Machine(m *ik.Machine) (*{prefix}Machine, error) {{\n\
         \tif err := AssertMachine(m); err != nil {{\n\
         \t\treturn nil, err\n\
         \t}}\n\
         \treturn &{prefix}Machine{{Raw: m}}, nil\n\
         }}\n\n\
         func (m *{prefix}Machine) Create(ctx context.Context, aggregateID string) (*{prefix}Aggregate, error) {{\n\
         \tagg, err := m.Raw.Create(ctx, aggregateID)\n\
         \tif err != nil {{\n\
         \t\treturn nil, err\n\
         \t}}\n\
         \treturn &{prefix}Aggregate{{Raw: agg}}, nil\n\
         }}\n\n\
         func (m *{prefix}Machine) Show(ctx context.Context, aggregateID string) (*{prefix}Aggregate, error) {{\n\
         \tagg, err := m.Raw.Show(ctx, aggregateID)\n\
         \tif err != nil {{\n\
         \t\treturn nil, err\n\
         \t}}\n\
         \treturn &{prefix}Aggregate{{Raw: agg}}, nil\n\
         }}\n\n\
         type {prefix}Aggregate struct {{ Raw *ik.Aggregate }}\n\n\
         func (a *{prefix}Aggregate) StateName() StateName {{ return StateName(a.Raw.StateName()) }}\n"
    ));

    for (name, cmd) in &def.commands {
        let method = pascal_case(name);
        let args = cmd.args.as_ref();
        if let Some(map) = args.filter(|m| !m.is_empty()) {
            out.push_str(&format!("\ntype {prefix}{method}Args struct {{\n"));
            for (k, ty) in map {
                let go_ty = match ty.as_str() {
                    "int" => "int64",
                    "bool" => "bool",
                    _ => "string",
                };
                out.push_str(&format!("\t{} {go_ty}\n", pascal_case(k)));
            }
            out.push_str("}\n\n");
            out.push_str(&format!(
                "func (a *{prefix}Aggregate) {method}(ctx context.Context, operationID string, args {prefix}{method}Args) (*{prefix}Aggregate, *ik.Historical, error) {{\n\
                 \twire := map[string]any{{\n"
            ));
            for k in map.keys() {
                out.push_str(&format!("\t\t{k:?}: args.{},\n", pascal_case(k)));
            }
            out.push_str(&format!(
                "\t}}\n\
                 \tnext, hist, err := a.Raw.CommandWithOperation(ctx, operationID, {name:?}, wire)\n\
                 \tif err != nil || hist != nil {{\n\
                 \t\treturn nil, hist, err\n\
                 \t}}\n\
                 \treturn &{prefix}Aggregate{{Raw: next}}, nil, nil\n\
                 }}\n"
            ));
        } else {
            out.push_str(&format!(
                "\nfunc (a *{prefix}Aggregate) {method}(ctx context.Context, operationID string) (*{prefix}Aggregate, *ik.Historical, error) {{\n\
                 \tnext, hist, err := a.Raw.CommandWithOperation(ctx, operationID, {name:?}, nil)\n\
                 \tif err != nil || hist != nil {{\n\
                 \t\treturn nil, hist, err\n\
                 \t}}\n\
                 \treturn &{prefix}Aggregate{{Raw: next}}, nil, nil\n\
                 }}\n"
            ));
        }
    }
    out
}
