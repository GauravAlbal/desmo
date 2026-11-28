//! `ik` CLI: strict canonical JSON envelopes over stdin/stdout.
//!
//! Usage: `ik --store <path> [--machine <def.toml>] create|command|record|show|replay|verify`
//!
//! `ik scenario` is a separate entry and does not read a caller store path.
//!
//! Reads one request envelope from stdin, writes one response envelope to stdout.
//! Exit codes: 0 success, 1 envelope-level refusal, 2 transport/configuration failure.

use crate::canonical::{canonical_json, normalize_shared_numbers};
use crate::declarative::{load_def, load_definition};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::external::ExternalMachine;
use crate::identity::AggregateId;
use crate::machine::{Machine, MachineRegistry, RegisteredMachine};
use crate::operation::{Lookup, OperationHistory, OperationId, OperationOutcome, OperationRequest};
use crate::protocol::{EnvelopeRequest, EnvelopeResponse, Op, RUNTIME_VERSION};
use crate::store::{
    CommandRequest, CommandResult, EffectOutcomeRequest, Store, TransactionRequest,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn entry() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    run(&args)
}

struct Subcommand {
    group: &'static str,
    name: &'static str,
    usage: &'static str,
    run: fn(&[String]) -> i32,
}

/// Dispatch table. `ik --help` lists these names in this exact order, grouped by `group`.
const SUBCOMMANDS: &[Subcommand] = &[
    Subcommand {
        group: "Quickstart",
        name: "demo",
        usage: "       ik demo [orders|stale-worker] [--json]",
        run: crate::quickstart::run_demo,
    },
    Subcommand {
        group: "Quickstart",
        name: "init",
        usage: "       ik init [order|job] --lang ts|py|go [--out <dir>] [--force]",
        run: crate::quickstart::run_init,
    },
    Subcommand {
        group: "Quickstart",
        name: "doctor",
        usage: "       ik doctor [--dir <path>] [--json]",
        run: crate::quickstart::run_doctor,
    },
    Subcommand {
        group: "Model & Verify",
        name: "diagram",
        usage: "       ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json] [--focus <state>] [--level 0|1|2] [--ascii] [--operation <id>]",
        run: crate::diagram::run,
    },
    Subcommand {
        group: "Model & Verify",
        name: "simulate",
        usage: "       ik simulate <machine.ik.toml> [--out <file.html>] [--open] [--check] [--no-check] [--corpus <cases.json>] [--bound <expr>] [--simulate] [--seed <u64>] [--steps <n>]",
        run: crate::simulate::run,
    },
    Subcommand {
        group: "Model & Verify",
        name: "check",
        usage: "       ik check <machine.ik.toml> [--prior <old.toml>] [--corpus <cases.json>] [--auto-seed] [--corpus-template <machine.ik.toml>] [--baseline <json>] [--release] [--summary] [--json] [--compact] [--region-product-bound <n>] [--profile <checks.toml>] [--plan] [--bound <expr>] [--simulate] [--seed <u64>] [--steps <n>] [--traces <n>] [--threads <n>] [--fail-fast] [--emit-scenario <file.json>]",
        run: run_check,
    },
    Subcommand {
        group: "Model & Verify",
        name: "codegen",
        usage: "       ik codegen <machine.ik.toml> --lang ts|py|go [--out <file>] [--check] [--import-from <specifier>] [--package <name>]",
        run: crate::codegen::run,
    },
    Subcommand {
        group: "Model & Verify",
        name: "laws",
        usage: "       ik laws <machine.ik.toml>",
        run: run_laws,
    },
    Subcommand {
        group: "Model & Verify",
        name: "corpus",
        usage: "       ik corpus init <machine.ik.toml> --out <cases.json> [--smart]",
        run: run_corpus,
    },
    Subcommand {
        group: "Model & Verify",
        name: "scenario",
        usage: "       ik scenario run <machine.ik.toml> <scenario.json> --out <dir> | reproduce <bundle-dir> --out <dir>",
        run: crate::scenario::run,
    },
    Subcommand {
        group: "Model & Verify",
        name: "guard",
        usage: "       ik guard init [--machine <m.ik.toml>] [--table <name>] [--out <path>] [--force] | check [--config <path>] [--dir <path>] [--json]",
        run: crate::guard::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "identity",
        usage: "       ik identity <machine.ik.toml>",
        run: run_identity,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "schema",
        usage: "       ik schema <machine.ik.toml>",
        run: run_schema,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "operations",
        usage: "       ik operations <store.sqlite> --aggregate <id> [--limit <n>] [--json]",
        run: crate::operations::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "events",
        usage: "       ik events <store.sqlite> [--since <cursor>] [--aggregate <id>] [--limit <n>] [--json]",
        run: crate::events::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "list",
        usage: "       ik list <store.sqlite> [--machine <id>] [--state <name>] [--effect none|pending|recorded] [--terminal true|false] [--after <aggregate_id>] [--limit <n>] [--json]",
        run: crate::list::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "census",
        usage: "       ik census <store.sqlite> [--json]",
        run: crate::census::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "store",
        usage: "       ik store verify <store.sqlite> [--machine <m.ik.toml>]... [--json]",
        run: crate::store_verify::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "report",
        usage: "       ik report <path>... [--json] [--html <file>]",
        run: crate::report::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "handoff",
        usage: "       ik handoff <behavior.json> --evidence <path>... [--kind delivery|change|recovery] [--store <store.sqlite>] [--json]",
        run: crate::handoff::run,
    },
    Subcommand {
        group: "Inspect & Store",
        name: "readout",
        usage: "       ik readout <summary.md> --handoff <handoff.txt> [--stamp] [--json]",
        run: crate::readout::run,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "regression",
        usage: "       ik regression promote <bundle-dir> --name <name> --reason <reason> [--into <dir>] | run <dir>",
        run: crate::regression::run,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "profile",
        usage: "       ik profile diff <old.toml> <new.toml> [--json]",
        run: run_profile,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "evidence",
        usage: "       ik evidence verify <evidence.json> --machine <m.ik.toml> [--profile <p.toml>] [--corpus <c.json>] [--runtime-binary <path>] [--json]",
        run: run_evidence,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "graph",
        usage: "       ik graph export <machine.ik.toml> [--corpus cases.json] [--max-nodes N] [--max-depth D] --out graph.json",
        run: crate::graph_export::run,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "analysis",
        usage: "       ik analysis describe|run|validate-result|conformance ...",
        run: crate::analysis::run_cli,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "smt",
        usage: "       ik smt --machine <def.toml> [--target <obligation_id>]",
        run: crate::smt::run_smt_cli,
    },
    Subcommand {
        group: "Advanced & Adapters",
        name: "feasibility-backend",
        usage: "       ik feasibility-backend --machine <def.toml> [--engine in-house|smt]",
        run: crate::smt::run_feasibility_backend,
    },
];

/// Names the dispatcher accepts, in the order `ik --help` lists them.
pub fn dispatched_subcommand_names() -> impl Iterator<Item = &'static str> {
    SUBCOMMANDS.iter().map(|command| command.name)
}

fn write_usage(emit: impl FnMut(&str)) {
    write_usage_filtered(false, emit);
}

fn write_usage_filtered(include_advanced: bool, mut emit: impl FnMut(&str)) {
    emit("Desmo (ik) — find hidden state bugs and enforce state machines at runtime");
    let mut current_group = "";
    let mut advanced_count = 0usize;
    for command in SUBCOMMANDS {
        if !include_advanced && command.group == "Advanced & Adapters" {
            advanced_count += 1;
            continue;
        }
        if command.group != current_group {
            emit("");
            emit(&format!("{}:", command.group));
            current_group = command.group;
        }
        emit(command.usage);
    }
    if !include_advanced && advanced_count > 0 {
        emit("");
        emit(&format!(
            "Advanced & Adapters: ({advanced_count} hidden — run `ik --help-all` to show all commands)"
        ));
    }
    emit("");
    emit("Runtime (JSON wire / persistent session):");
    emit("usage: ik --store <path> [--machine <def.toml>] [--session] create|create_with_command|command|record|show|replay|verify|transaction|lookup_operation");
    emit("       ik --help");
    emit("       ik --help-all");
    emit("       ik --version");
}

/// Dispatch the named subcommands; `None` means the legacy envelope mode.
fn run_subcommand(args: &[String]) -> Option<i32> {
    let rest = args.get(1..)?;
    let name = args.first()?.as_str();
    SUBCOMMANDS
        .iter()
        .find(|command| command.name == name)
        .map(|command| (command.run)(rest))
}

/// Run one CLI invocation. Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        write_usage(|line| println!("{line}"));
        return 0;
    }
    if let Some(code) = run_subcommand(args) {
        return code;
    }
    let parsed = match parse_args(args) {
        Ok(Some(flags)) => flags,
        Ok(None) => return 0, // --version/--help handled
        Err(msg) => return argument_error(&msg),
    };
    if parsed.session {
        return run_session(&parsed);
    }
    let mut req = match read_request() {
        Ok(req) => req,
        Err(code) => return code,
    };
    if let Err(diagnostic) = validate_request(&mut req, &parsed) {
        return emit_failure(&req.request_id, diagnostic);
    }

    let mut registry = MachineRegistry::new();
    let registration = match register_reducer(&req, &parsed, &mut registry) {
        Ok(registration) => registration,
        Err(RegistrationFailure::Configuration(errors)) => {
            for error in errors {
                eprintln!("ik: machine definition error: {error}");
            }
            return 2;
        }
        Err(RegistrationFailure::Registration(diagnostic)) => {
            eprintln!("ik: machine registration failed: {diagnostic}");
            return 2;
        }
        Err(RegistrationFailure::Envelope(diagnostic)) => {
            return emit_failure(&req.request_id, diagnostic);
        }
    };
    if let Some(positional) = &parsed.op {
        if let Err(diagnostic) = validate_positional_op(positional, &req.op) {
            return emit_failure(&req.request_id, diagnostic);
        }
    }

    let mut stores = StoreSlot::new(parsed.store.clone());
    let response = dispatch(&req, &mut stores, &registry, registration.as_ref());
    println!("{}", canonical_json(&response));
    if response.ok {
        0
    } else {
        1
    }
}

fn argument_error(message: &str) -> i32 {
    eprintln!("ik: {message}");
    write_usage(|line| eprintln!("{line}"));
    2
}

fn request_id_of(value: &Value) -> String {
    value
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn protocol_failure(request_id: &str, message: String) -> EnvelopeResponse {
    EnvelopeResponse::failure(
        request_id,
        Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            crate::diagnostic::NO_RULE,
            message,
        ),
    )
}

/// Decode one envelope. A typed failure (partial effect, unknown field) still
/// echoes `request_id` from the JSON object so clients can classify the
/// diagnostic instead of treating an empty id as a transport mismatch.
fn decode_envelope(input: &str) -> Result<EnvelopeRequest, EnvelopeResponse> {
    let value: Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(error) => {
            return Err(protocol_failure("", format!("malformed request: {error}")));
        }
    };
    let request_id = request_id_of(&value);
    match serde_json::from_value(value) {
        Ok(req) => Ok(req),
        Err(error) => Err(protocol_failure(
            &request_id,
            format!("malformed request: {error}"),
        )),
    }
}

fn read_request() -> Result<EnvelopeRequest, i32> {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("ik: failed to read request from stdin");
        return Err(2);
    }
    match decode_envelope(&input) {
        Ok(req) => Ok(req),
        Err(response) => {
            println!("{}", canonical_json(&response));
            Err(1)
        }
    }
}

fn emit_failure(request_id: &str, diagnostic: Diagnostic) -> i32 {
    let response = EnvelopeResponse::failure(request_id, diagnostic);
    println!("{}", canonical_json(&response));
    1
}

/// Upper bound on steps in one `transaction`; keeps a single store transaction short.
const MAX_TRANSACTION_STEPS: usize = 64;

fn validate_request(req: &mut EnvelopeRequest, parsed: &Flags) -> Result<(), Diagnostic> {
    if [
        req.reducer_worker.as_mut(),
        req.command.as_mut(),
        req.observation.as_mut(),
    ]
    .into_iter()
    .flatten()
    .any(|value| !normalize_shared_numbers(value))
        || req.commands.iter_mut().flatten().any(|step| {
            !normalize_shared_numbers(&mut step.command)
                || step
                    .observation
                    .as_mut()
                    .is_some_and(|o| !normalize_shared_numbers(o))
        })
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("request.number"),
            "JSON integer is outside the shared range [-2^63, 2^64-1]",
        ));
    }
    match (&req.op, &req.commands) {
        (Op::Transaction, Some(steps))
            if !steps.is_empty()
                && steps.len() <= MAX_TRANSACTION_STEPS
                && req.command.is_none()
                && req.observation.is_none() => {}
        (Op::Transaction, _) => {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProtocolInvalid,
                Some("request.commands"),
                format!(
                    "`transaction` requires 1..={MAX_TRANSACTION_STEPS} `commands` and no top-level `command`/`observation`"
                ),
            ));
        }
        (Op::LookupOperation, _) => {}
        (_, Some(_)) => {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProtocolInvalid,
                Some("request.commands"),
                "`commands` is only valid on op `transaction`",
            ));
        }
        _ => {}
    }
    if req.protocol.major != crate::protocol::PROTOCOL_MAJOR {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolVersionUnknown,
            Some("protocol.major"),
            format!(
                "unknown protocol major {} (supported {})",
                req.protocol.major,
                crate::protocol::PROTOCOL_MAJOR
            ),
        ));
    }
    if req.runtime_version != RUNTIME_VERSION {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolVersionUnknown,
            Some("runtime.version"),
            format!(
                "unknown runtime version `{}` (supported `{RUNTIME_VERSION}`)",
                req.runtime_version
            ),
        ));
    }
    if parsed.machine.is_some() && req.reducer_worker.is_some() {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("reducer.source"),
            "`--machine` and request `reducer_worker` are mutually exclusive",
        ));
    }
    if matches!(req.op, Op::Record | Op::Show | Op::LookupOperation) && req.reducer_worker.is_some()
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("request.reducer_worker"),
            format!(
                "operation `{}` must not carry `reducer_worker`",
                req.op.as_str()
            ),
        ));
    }
    if req.operation_id.is_some()
        && !matches!(
            req.op,
            Op::Command
                | Op::Transaction
                | Op::Record
                | Op::LookupOperation
                | Op::CreateWithCommand
        )
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("request.operation_id"),
            "`operation_id` is only valid on `command`, `transaction`, `record`, `lookup_operation`, and `create_with_command`",
        ));
    }
    Ok(())
}

enum RegistrationFailure {
    Configuration(Vec<String>),
    Registration(Diagnostic),
    Envelope(Diagnostic),
}

fn register_reducer(
    req: &EnvelopeRequest,
    parsed: &Flags,
    registry: &mut MachineRegistry,
) -> Result<Option<RegisteredMachine>, RegistrationFailure> {
    if matches!(req.op, Op::Record | Op::Show | Op::LookupOperation) {
        return Ok(None);
    }
    match (&parsed.machine, &req.reducer_worker) {
        (None, None) => Ok(None),
        (Some(path), None) => {
            let machine = load_def(path).map_err(RegistrationFailure::Configuration)?;
            registry
                .register(Box::new(machine))
                .map(Some)
                .map_err(RegistrationFailure::Registration)
        }
        (None, Some(spec)) => {
            let machine = ExternalMachine::from_json(&canonical_json(spec))
                .map_err(RegistrationFailure::Envelope)?;
            registry
                .register(Box::new(machine))
                .map(Some)
                .map_err(RegistrationFailure::Envelope)
        }
        (Some(_), Some(_)) => Err(RegistrationFailure::Envelope(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("reducer.source"),
            "`--machine` and request `reducer_worker` are mutually exclusive",
        ))),
    }
}

fn validate_positional_op(positional: &str, op: &Op) -> Result<(), Diagnostic> {
    if positional == op.as_str() {
        return Ok(());
    }
    Err(Diagnostic::new(
        DiagnosticCode::IkProtocolInvalid,
        Some("request.op"),
        format!(
            "positional operation `{positional}` disagrees with envelope op `{}`",
            op.as_str()
        ),
    ))
}

struct Flags {
    store: PathBuf,
    machine: Option<PathBuf>,
    op: Option<String>,
    session: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Flags>, String> {
    let mut store: Option<PathBuf> = None;
    let mut machine: Option<PathBuf> = None;
    let mut op: Option<String> = None;
    let mut session = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--store" => {
                i += 1;
                store = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "--store requires a path".to_string())?,
                ));
            }
            "--machine" => {
                i += 1;
                machine = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "--machine requires a path".to_string())?,
                ));
            }
            "--session" => session = true,
            "--version" => {
                println!(
                    "invariant-kernel {} protocol {}.{}",
                    RUNTIME_VERSION,
                    crate::protocol::PROTOCOL_MAJOR,
                    crate::protocol::PROTOCOL_MINOR
                );
                return Ok(None);
            }
            "-h" | "--help" => {
                write_usage(|line| println!("{line}"));
                return Ok(None);
            }
            "--help-all" => {
                write_usage_filtered(true, |line| println!("{line}"));
                return Ok(None);
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            name if op.is_none() => op = Some(name.to_string()),
            _ => return Err(format!("unexpected argument `{}`", args[i])),
        }
        i += 1;
    }
    let store = store.ok_or_else(|| "--store <path> is required".to_string())?;
    Ok(Some(Flags {
        store,
        machine,
        op,
        session,
    }))
}

/// One lazily opened store per process or session. One-shot and session mode share this single
/// dispatch path; the only difference is how long the store (and reducer worker) stay open.
struct StoreSlot {
    path: PathBuf,
    store: Option<Store>,
}

impl StoreSlot {
    fn new(path: PathBuf) -> Self {
        Self { path, store: None }
    }

    fn open(&mut self, create: bool) -> Result<&mut Store, Diagnostic> {
        if self.store.is_none() {
            self.store = Some(if create {
                Store::open_or_create(&self.path)?
            } else {
                Store::open(&self.path)?
            });
        }
        self.store
            .as_mut()
            .ok_or_else(|| Diagnostic::internal("store slot empty after open"))
    }
}

/// `ik --session`: newline-delimited canonical JSON requests on stdin, one response line each.
/// The reducer is registered once and bound for the session; each request is still validated and
/// dispatched independently, so a failed request never corrupts the session.
fn run_session(parsed: &Flags) -> i32 {
    use std::io::{BufRead, Write};
    let mut registry = MachineRegistry::new();
    let mut cli_machine: Option<RegisteredMachine> = None;
    if let Some(path) = &parsed.machine {
        match load_def(path) {
            Ok(machine) => match registry.register(Box::new(machine)) {
                Ok(registration) => cli_machine = Some(registration),
                Err(diagnostic) => {
                    eprintln!("ik: machine registration failed: {diagnostic}");
                    return 2;
                }
            },
            Err(errors) => {
                for error in errors {
                    eprintln!("ik: machine definition error: {error}");
                }
                return 2;
            }
        }
    }
    let mut bound: Option<(String, RegisteredMachine)> = None;
    let mut stores = StoreSlot::new(parsed.store.clone());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else {
            eprintln!("ik: failed to read session request");
            return 2;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = session_request(
            &line,
            parsed,
            &mut registry,
            &mut bound,
            cli_machine.as_ref(),
            &mut stores,
        );
        if let Err(err) = writeln!(out, "{}", canonical_json(&response)) {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                return 0;
            }
            eprintln!("ik: failed to write session response: {err}");
            return 2;
        }
        if let Err(err) = out.flush() {
            if err.kind() == std::io::ErrorKind::BrokenPipe {
                return 0;
            }
            eprintln!("ik: failed to flush session response: {err}");
            return 2;
        }
    }
    0
}

fn session_request(
    line: &str,
    parsed: &Flags,
    registry: &mut MachineRegistry,
    bound: &mut Option<(String, RegisteredMachine)>,
    cli_machine: Option<&RegisteredMachine>,
    stores: &mut StoreSlot,
) -> EnvelopeResponse {
    let mut req = match decode_envelope(line) {
        Ok(req) => req,
        Err(response) => return response,
    };
    if let Err(diagnostic) = validate_request(&mut req, parsed) {
        return EnvelopeResponse::failure(&req.request_id, diagnostic);
    }
    if !matches!(req.op, Op::Record | Op::Show | Op::LookupOperation) {
        if let Some(spec) = &req.reducer_worker {
            let key = canonical_json(spec);
            match bound {
                Some((bound_key, _)) if *bound_key != key => {
                    return EnvelopeResponse::failure(
                        &req.request_id,
                        Diagnostic::new(
                            DiagnosticCode::IkMachineMismatch,
                            Some("session.reducer"),
                            "session is already bound to a different reducer",
                        ),
                    );
                }
                Some(_) => {}
                None => {
                    let registration = ExternalMachine::from_json(&key)
                        .and_then(|machine| registry.register(Box::new(machine)));
                    match registration {
                        Ok(registration) => *bound = Some((key, registration)),
                        Err(diagnostic) => {
                            return EnvelopeResponse::failure(&req.request_id, diagnostic);
                        }
                    }
                }
            }
        }
    }
    let registration = if matches!(req.op, Op::Record | Op::Show | Op::LookupOperation) {
        None
    } else {
        bound
            .as_ref()
            .map(|(_, registration)| registration)
            .or(cli_machine)
    };
    dispatch(&req, stores, registry, registration)
}

fn dispatch(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> EnvelopeResponse {
    match dispatch_inner(req, stores, registry, registration) {
        Ok(result) => EnvelopeResponse::success(&req.request_id, result),
        Err(diagnostic) => EnvelopeResponse::failure(&req.request_id, diagnostic),
    }
}

fn required<T>(req: &EnvelopeRequest, field: &str, value: Option<T>) -> Result<T, Diagnostic> {
    value.ok_or_else(|| {
        Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some(format!("request.{:?}.{}", req.op, field)),
            format!("request for op `{:?}` requires `{field}`", req.op),
        )
    })
}

fn dispatch_inner(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    match req.op {
        Op::Create => op_create(req, stores, registry, registration),
        Op::Command => op_command(req, stores, registry, registration),
        Op::Record => op_record(req, stores),
        Op::Show => op_show(req, stores),
        Op::Replay => op_replay(req, stores, registry, registration),
        Op::Verify => op_verify(req, stores, registry, registration),
        Op::Transaction => op_transaction(req, stores, registry, registration),
        Op::LookupOperation => op_lookup_operation(req, stores),
        Op::CreateWithCommand => op_create_with_command(req, stores, registry, registration),
    }
}

/// Resolve the machine only after a successful registration has produced its
/// unforgeable token. Every Store operation receives a machine from here.
fn registered_machine<'a>(
    req: &EnvelopeRequest,
    registry: &'a MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<&'a dyn Machine, Diagnostic> {
    let registration = required(req, "machine", registration)?;
    registry.resolve(registration, registration.identity())
}

fn op_create(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    let machine = registered_machine(req, registry, registration)?;
    let aggregate_id = required(req, "aggregate_id", req.aggregate_id.clone())?;
    let presented = required(req, "machine", req.machine.clone())?;
    let store = stores.open(true)?;
    let result = store.create_aggregate(machine, &AggregateId(aggregate_id.clone()), &presented)?;
    Ok(command_result_view(&result, &aggregate_id))
}

fn op_create_with_command(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    reject_create_with_extras(req)?;
    let machine = registered_machine(req, registry, registration)?;
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let presented = required(req, "machine", req.machine.clone())?;
    let command = required(req, "command", req.command.clone())?;
    let operation_id = operation_id_of(req)?;
    let store = stores.open(true)?;
    admit_with_command(
        store,
        machine,
        &aggregate_id,
        &presented,
        &command,
        req,
        operation_id,
    )
}

fn reject_create_with_extras(req: &EnvelopeRequest) -> Result<(), Diagnostic> {
    if req.capability.is_none() && req.commands.is_none() && req.effect.is_none() {
        return Ok(());
    }
    Err(Diagnostic::new(
        DiagnosticCode::IkProtocolInvalid,
        Some("request.create_with_command"),
        "`create_with_command` requires `aggregate_id` and `command` and must not carry `capability`, `commands`, or `effect`",
    ))
}

fn admit_with_command(
    store: &mut Store,
    machine: &dyn crate::machine::Machine,
    aggregate_id: &AggregateId,
    presented: &crate::identity::MachineIdentity,
    command: &Value,
    req: &EnvelopeRequest,
    operation_id: Option<OperationId>,
) -> Result<Value, Diagnostic> {
    let observation = req.observation.as_ref();
    let Some(operation_id) = operation_id else {
        let outcome = store.create_aggregate_with_command(
            machine,
            aggregate_id,
            presented,
            command,
            observation,
        )?;
        return Ok(admission_view(aggregate_id, outcome));
    };
    match store.create_aggregate_with_command_op(
        machine,
        aggregate_id,
        presented,
        command,
        observation,
        &operation_id,
    )? {
        OperationOutcome::Committed(outcome) => Ok(admission_view(aggregate_id, outcome)),
        OperationOutcome::Historical(history) => Ok(history_view(&aggregate_id.0, &history)),
    }
}

fn admission_view(
    aggregate_id: &AggregateId,
    outcome: crate::store::CreateWithCommandOutcome,
) -> Value {
    use crate::store::CreateWithCommandOutcome;
    match outcome {
        CreateWithCommandOutcome::Committed(result)
        | CreateWithCommandOutcome::ExactDuplicate(result) => {
            command_result_view(&result, &aggregate_id.0)
        }
    }
}

fn operation_id_of(req: &EnvelopeRequest) -> Result<Option<OperationId>, Diagnostic> {
    req.operation_id
        .as_deref()
        .map(OperationId::parse)
        .transpose()
}

fn op_command(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    let machine = registered_machine(req, registry, registration)?;
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let presented = required(req, "machine", req.machine.clone())?;
    let capability = required(req, "capability", req.capability.clone())?;
    let command = required(req, "command", req.command.clone())?;
    let operation_id = operation_id_of(req)?;
    let store = stores.open(false)?;
    let request = CommandRequest {
        aggregate_id: aggregate_id.clone(),
        machine: presented,
        capability,
        command,
        observation: req.observation.clone(),
    };
    let Some(operation_id) = operation_id else {
        let result = store.command(machine, &request)?;
        return Ok(command_result_view(&result, &aggregate_id.0));
    };
    match store.command_op(machine, &request, &operation_id)? {
        OperationOutcome::Committed(result) => Ok(command_result_view(&result, &aggregate_id.0)),
        OperationOutcome::Historical(history) => Ok(history_view(&aggregate_id.0, &history)),
    }
}

fn op_transaction(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    let machine = registered_machine(req, registry, registration)?;
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let presented = required(req, "machine", req.machine.clone())?;
    let capability = required(req, "capability", req.capability.clone())?;
    let steps = required(req, "commands", req.commands.clone())?;
    let operation_id = operation_id_of(req)?;
    let store = stores.open(false)?;
    let request = TransactionRequest {
        aggregate_id: aggregate_id.clone(),
        machine: presented,
        capability,
        steps,
    };
    let results = match operation_id {
        None => store.transaction(machine, &request)?,
        Some(operation_id) => match store.transaction_op(machine, &request, &operation_id)? {
            OperationOutcome::Committed(results) => results,
            OperationOutcome::Historical(history) => {
                return Ok(history_view(&aggregate_id.0, &history));
            }
        },
    };
    let views: Vec<Value> = results
        .iter()
        .map(|result| command_result_view(result, &aggregate_id.0))
        .collect();
    let mut view = views.last().cloned().unwrap_or(Value::Null);
    if let Some(map) = view.as_object_mut() {
        map.insert("steps".to_string(), Value::Array(views));
    }
    Ok(view)
}

fn op_record(req: &EnvelopeRequest, stores: &mut StoreSlot) -> Result<Value, Diagnostic> {
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let presented = required(req, "machine", req.machine.clone())?;
    let capability = required(req, "capability", req.capability.clone())?;
    let effect = required(req, "effect", req.effect.clone())?;
    let operation_id = operation_id_of(req)?;
    let store = stores.open(false)?;
    let request = EffectOutcomeRequest {
        aggregate_id: aggregate_id.clone(),
        machine: presented,
        capability,
        effect,
    };
    let Some(operation_id) = operation_id else {
        let result = store.record_effect(&request)?;
        return Ok(command_result_view(&result, &aggregate_id.0));
    };
    match store.record_effect_op(&request, &operation_id)? {
        OperationOutcome::Committed(result) => Ok(command_result_view(&result, &aggregate_id.0)),
        OperationOutcome::Historical(history) => Ok(history_view(&aggregate_id.0, &history)),
    }
}

/// Read-only `lookup_operation`: the same body as the mutation it names, executed
/// by nothing. The request kind is inferred from which body fields are present.
fn op_lookup_operation(req: &EnvelopeRequest, stores: &mut StoreSlot) -> Result<Value, Diagnostic> {
    let operation_id = required(req, "operation_id", operation_id_of(req)?)?;
    let store = stores.open(false)?;
    let lookup = if req.commands.is_some() {
        lookup_transaction(req, store, &operation_id)?
    } else if req.effect.is_some() {
        lookup_record(req, store, &operation_id)?
    } else if req.command.is_some() && req.capability.is_none() {
        lookup_create_with(req, store, &operation_id)?
    } else {
        lookup_command(req, store, &operation_id)?
    };
    serde_json::to_value(lookup)
        .map_err(|e| Diagnostic::internal(format!("lookup encode failed: {e}")))
}

fn lookup_transaction(
    req: &EnvelopeRequest,
    store: &mut Store,
    operation_id: &OperationId,
) -> Result<Lookup, Diagnostic> {
    let request = TransactionRequest {
        aggregate_id: AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?),
        machine: required(req, "machine", req.machine.clone())?,
        capability: required(req, "capability", req.capability.clone())?,
        steps: required(req, "commands", req.commands.clone())?,
    };
    store.lookup_operation(operation_id, &OperationRequest::Transaction(&request))
}

fn lookup_record(
    req: &EnvelopeRequest,
    store: &mut Store,
    operation_id: &OperationId,
) -> Result<Lookup, Diagnostic> {
    let request = EffectOutcomeRequest {
        aggregate_id: AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?),
        machine: required(req, "machine", req.machine.clone())?,
        capability: required(req, "capability", req.capability.clone())?,
        effect: required(req, "effect", req.effect.clone())?,
    };
    store.lookup_operation(operation_id, &OperationRequest::Record(&request))
}

fn lookup_create_with(
    req: &EnvelopeRequest,
    store: &mut Store,
    operation_id: &OperationId,
) -> Result<Lookup, Diagnostic> {
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let machine = required(req, "machine", req.machine.clone())?;
    let command = required(req, "command", req.command.clone())?;
    store.lookup_operation(
        operation_id,
        &OperationRequest::CreateWithCommand {
            machine: &machine,
            aggregate_id: &aggregate_id,
            command: &command,
            observation: req.observation.as_ref(),
        },
    )
}

fn lookup_command(
    req: &EnvelopeRequest,
    store: &mut Store,
    operation_id: &OperationId,
) -> Result<Lookup, Diagnostic> {
    let request = CommandRequest {
        aggregate_id: AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?),
        machine: required(req, "machine", req.machine.clone())?,
        capability: required(req, "capability", req.capability.clone())?,
        command: required(req, "command", req.command.clone())?,
        observation: req.observation.clone(),
    };
    store.lookup_operation(operation_id, &OperationRequest::Command(&request))
}

fn op_show(req: &EnvelopeRequest, stores: &mut StoreSlot) -> Result<Value, Diagnostic> {
    let aggregate_id = AggregateId(required(req, "aggregate_id", req.aggregate_id.clone())?);
    let store = stores.open(false)?;
    let projection = store.show(&aggregate_id)?;
    serde_json::to_value(projection)
        .map_err(|e| Diagnostic::internal(format!("projection encode failed: {e}")))
}

fn op_replay(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    let machine = registered_machine(req, registry, registration)?;
    let store = stores.open(false)?;
    let id = req.aggregate_id.clone().map(AggregateId);
    let summary = store.replay(machine, id.as_ref())?;
    serde_json::to_value(summary)
        .map_err(|e| Diagnostic::internal(format!("replay summary encode failed: {e}")))
}

fn op_verify(
    req: &EnvelopeRequest,
    stores: &mut StoreSlot,
    registry: &MachineRegistry,
    registration: Option<&RegisteredMachine>,
) -> Result<Value, Diagnostic> {
    let machine = registered_machine(req, registry, registration)?;
    let store = stores.open(false)?;
    let id = req.aggregate_id.clone().map(AggregateId);
    let summary = store.verify(machine, id.as_ref())?;
    serde_json::to_value(summary)
        .map_err(|e| Diagnostic::internal(format!("verify summary encode failed: {e}")))
}

fn command_result_view(result: &CommandResult, aggregate_id: &str) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "aggregate_id".to_string(),
        Value::String(aggregate_id.to_string()),
    );
    map.insert("generation".to_string(), json!(result.generation));
    map.insert("state".to_string(), result.state.clone());
    map.insert("terminal".to_string(), json!(result.terminal));
    map.insert(
        "capability".to_string(),
        serde_json::to_value(&result.capability).unwrap_or(Value::Null),
    );
    map.insert(
        "effect".to_string(),
        serde_json::to_value(&result.effect).unwrap_or(Value::Null),
    );
    map.insert("mutation".to_string(), json!(result.mutation));
    if let Some(receipt) = &result.receipt {
        map.insert(
            "receipt".to_string(),
            serde_json::to_value(receipt).unwrap_or(Value::Null),
        );
    }
    if let Some(note) = &result.note {
        map.insert("note".to_string(), note.clone());
    }
    Value::Object(map)
}

/// An exact repeat of a committed operation. It reports what was recorded and
/// the aggregate's current coordinates; it carries no state and no capability,
/// so it cannot be mistaken for a current handle.
fn history_view(aggregate_id: &str, history: &OperationHistory) -> Value {
    json!({
        "aggregate_id": aggregate_id,
        "historical": true,
        "mutation": false,
        "operation": history.record,
        "current": {
            "generation": history.current_generation,
            "sequence": history.current_sequence,
        },
    })
}

fn run_identity(args: &[String]) -> i32 {
    match run_identity_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik identity: {msg}");
            eprintln!("usage: ik identity <machine.ik.toml>");
            2
        }
    }
}

fn run_identity_inner(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            println!("usage: ik identity <machine.ik.toml>");
            Ok(0)
        }
        None => Err("machine definition path is required".into()),
        Some(flag) if flag.starts_with('-') => Err(format!("unknown flag `{flag}`")),
        Some(_) if args.len() > 1 => Err(format!("unexpected argument `{}`", args[1])),
        Some(path) => {
            let machine = load_def(Path::new(path)).map_err(|errors| errors.join("; "))?;
            println!("{}", canonical_json(machine.identity()));
            Ok(0)
        }
    }
}

fn run_schema(args: &[String]) -> i32 {
    match run_schema_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik schema: {msg}");
            eprintln!("usage: ik schema <machine.ik.toml>");
            2
        }
    }
}

/// Machine-derived schema metadata for clients and code generators (`ikaros.machine-schema.v1`).
fn run_schema_inner(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            println!("usage: ik schema <machine.ik.toml>");
            Ok(0)
        }
        None => Err("machine definition path is required".into()),
        Some(flag) if flag.starts_with('-') => Err(format!("unknown flag `{flag}`")),
        Some(_) if args.len() > 1 => Err(format!("unexpected argument `{}`", args[1])),
        Some(path) => {
            let compiled = load_def(Path::new(path)).map_err(|errors| errors.join("; "))?;
            let schema =
                crate::declarative::schema::machine_schema(compiled.definition(), compiled.identity());
            println!("{}", canonical_json(&schema));
            Ok(0)
        }
    }
}

fn run_laws(args: &[String]) -> i32 {
    match run_laws_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik laws: {msg}");
            eprintln!("usage: ik laws <machine.ik.toml>");
            2
        }
    }
}

fn run_laws_inner(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            println!("usage: ik laws <machine.ik.toml>");
            Ok(0)
        }
        None => Err("machine definition path is required".into()),
        Some(flag) if flag.starts_with('-') => Err(format!("unknown flag `{flag}`")),
        Some(_) if args.len() > 1 => Err(format!("unexpected argument `{}`", args[1])),
        Some(path) => {
            let compiled = load_def(Path::new(path)).map_err(|errors| errors.join("; "))?;
            let def = compiled.definition();
            let identity = compiled.identity();
            let plan = crate::declarative::verification_plan(def);
            let laws: Vec<Value> = def
                .invariants
                .iter()
                .map(|(name, inv)| {
                    json!({
                        "id": name,
                        "when": inv.when,
                        "assert": inv.assert,
                        "enforcement": "runtime",
                        "evidence": plan
                            .obligations
                            .iter()
                            .find(|o| {
                                o.kind == "OBL-INV-001"
                                    && o.coordinate.invariant.as_deref() == Some(name.as_str())
                            })
                            .map(|o| json!({
                                "obligation_id": o.obligation_id,
                                "kind": o.kind,
                                "status": "not_examined",
                                "reason": "`ik check` has no verifier for this obligation kind; runtime enforcement is the only evidence",
                            })),
                        "identity": "bound to this machine version",
                    })
                })
                .collect();
            let commands: Vec<Value> = def
                .commands
                .keys()
                .map(|name| json!({ "name": name, "enforcement": "runtime" }))
                .collect();
            let lints = crate::declarative::structural_lints(def, &compiled);
            println!(
                "{}",
                canonical_json(&json!({
                    "advisory": true,
                    "machine_id": identity.machine_id,
                    "version": identity.version,
                    "spec_hash": identity.spec_hash,
                    "laws": laws,
                    "commands": commands,
                    "unguarded_arguments": crate::declarative::gaps::unguarded_arguments(def),
                    "structural_lints": lints,
                }))
            );
            Ok(0)
        }
    }
}

fn run_check(args: &[String]) -> i32 {
    match run_check_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik check: {msg}");
            eprintln!("usage: ik check <machine.ik.toml> [--prior <old.toml>] [--corpus <cases.json>] [--auto-seed] [--baseline <json>] [--release] [--summary] [--json] [--compact] [--region-product-bound <n>] [--profile <checks.toml>] [--plan] [--bound <expr>] [--simulate] [--seed <u64>] [--steps <n>] [--traces <n>] [--threads <n>] [--fail-fast] [--emit-scenario <file.json>]");
            2
        }
    }
}

struct CheckFlags {
    path: PathBuf,
    prior: Option<PathBuf>,
    corpus: Option<PathBuf>,
    baseline: Option<PathBuf>,
    profile: Option<PathBuf>,
    auto_seed: bool,
    release: bool,
    summary: bool,
    json: bool,
    compact: bool,
    plan: bool,
    region_product_bound: Option<usize>,
    template_only: bool,
    bounds_exprs: Vec<String>,
    simulate: bool,
    sim_seed: Option<u64>,
    sim_steps: Option<usize>,
    sim_traces: Option<usize>,
    threads: usize,
    fail_fast: bool,
    emit_scenario: Option<PathBuf>,
}

fn path_value(args: &[String], i: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a path"))
}

fn positive_usize(args: &[String], i: usize, flag: &str) -> Result<usize, String> {
    let raw = args
        .get(i)
        .ok_or_else(|| format!("{flag} requires a positive integer"))?;
    let parsed = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid {flag} `{raw}`"))?;
    if parsed == 0 {
        return Err(format!("{flag} must be >= 1"));
    }
    Ok(parsed)
}

fn parse_check_args(args: &[String]) -> Result<Option<CheckFlags>, String> {
    let mut path: Option<PathBuf> = None;
    let mut values: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let (mut auto_seed, mut release, mut summary, mut json, mut compact, mut plan) =
        (false, false, false, false, false, false);
    let (mut simulate, mut fail_fast) = (false, false);
    let mut region_product_bound: Option<usize> = None;
    let mut template: Option<PathBuf> = None;
    let mut emit_scenario: Option<PathBuf> = None;
    let mut bounds_exprs: Vec<String> = Vec::new();
    let mut sim_seed: Option<u64> = None;
    let mut sim_steps: Option<usize> = None;
    let mut sim_traces: Option<usize> = None;
    let mut threads: usize = 1;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("usage: ik check <machine.ik.toml> [--prior <old.toml>] [--corpus <cases.json>] [--auto-seed] [--baseline <json>] [--release] [--summary] [--json] [--compact] [--region-product-bound <n>] [--profile <checks.toml>] [--plan] [--bound <expr>] [--simulate] [--seed <u64>] [--steps <n>] [--traces <n>] [--threads <n>] [--fail-fast] [--emit-scenario <file.json>]");
                return Ok(None);
            }
            flag @ ("--prior" | "--corpus" | "--baseline" | "--profile") => {
                i += 1;
                values.insert(flag, path_value(args, i, flag)?);
            }
            "--emit-scenario" => {
                i += 1;
                emit_scenario = Some(path_value(args, i, "--emit-scenario")?);
            }
            "--corpus-template" => {
                i += 1;
                template = Some(path_value(args, i, "--corpus-template")?);
            }
            "--region-product-bound" => {
                i += 1;
                region_product_bound = Some(positive_usize(args, i, "--region-product-bound")?);
            }
            "--bound" => {
                i += 1;
                let expr = args
                    .get(i)
                    .ok_or("--bound requires an expression")?;
                bounds_exprs.push(expr.clone());
            }
            "--simulate" => simulate = true,
            "--seed" => {
                i += 1;
                let raw = args.get(i).ok_or("--seed requires an integer")?;
                sim_seed = Some(raw.parse::<u64>().map_err(|_| format!("invalid --seed `{raw}`"))?);
                simulate = true;
            }
            "--steps" => {
                i += 1;
                sim_steps = Some(positive_usize(args, i, "--steps")?);
                simulate = true;
            }
            "--traces" => {
                i += 1;
                sim_traces = Some(positive_usize(args, i, "--traces")?);
                simulate = true;
            }
            "--threads" => {
                i += 1;
                threads = positive_usize(args, i, "--threads")?;
            }
            "--fail-fast" => fail_fast = true,
            "--auto-seed" => auto_seed = true,
            "--release" => release = true,
            "--summary" => summary = true,
            "--json" => json = true,
            "--compact" => {
                compact = true;
                json = true;
            }
            "--plan" => plan = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            name if path.is_none() => path = Some(PathBuf::from(name)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    let template_only = template.is_some();
    let path = match (template, path) {
        (Some(_), Some(_)) => {
            return Err(
                "--corpus-template takes the machine path; do not also pass a positional".into(),
            );
        }
        (Some(template), None) => template,
        (None, Some(path)) => path,
        (None, None) => return Err("machine definition path is required".into()),
    };
    if auto_seed && release {
        return Err(
            "--auto-seed cannot be combined with --release; supply an explicit --corpus for release checks"
                .into(),
        );
    }
    if simulate && release {
        return Err(
            "--simulate cannot be combined with --release; supply an explicit --corpus for release checks"
                .into(),
        );
    }
    if auto_seed && values.contains_key("--corpus") {
        return Err("--auto-seed and --corpus are mutually exclusive".into());
    }
    Ok(Some(CheckFlags {
        path,
        prior: values.remove("--prior"),
        corpus: values.remove("--corpus"),
        baseline: values.remove("--baseline"),
        profile: values.remove("--profile"),
        auto_seed,
        release,
        summary,
        json,
        compact,
        plan,
        region_product_bound,
        template_only,
        bounds_exprs,
        simulate,
        sim_seed,
        sim_steps,
        sim_traces,
        threads,
        fail_fast,
        emit_scenario,
    }))
}

fn load_definition_file(
    path: &Path,
    limits: crate::declarative::region::RegionLimits,
) -> Result<crate::declarative::Definition, String> {
    load_source_file_with_limits(path, limits).map(|source| source.definition)
}

fn load_source_file(path: &Path) -> Result<crate::declarative::region::SourceDefinition, String> {
    load_source_file_with_limits(path, crate::declarative::region::RegionLimits::default())
}

fn load_source_file_with_limits(
    path: &Path,
    limits: crate::declarative::region::RegionLimits,
) -> Result<crate::declarative::region::SourceDefinition, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    crate::declarative::region::parse_source_with_limits(&src, limits)
        .map_err(|e| format!("parse {}: {}", path.display(), e.join("; ")))
}

fn run_check_inner(args: &[String]) -> Result<i32, String> {
    let Some(flags) = parse_check_args(args)? else {
        return Ok(0);
    };
    if flags.template_only {
        return emit_corpus_template(&flags.path);
    }
    if flags.plan && flags.profile.is_none() {
        return Err("--plan requires --profile".into());
    }
    let region_limits = match flags.region_product_bound {
        Some(region_product_bound) => {
            crate::declarative::region::RegionLimits { region_product_bound }
        }
        None => crate::declarative::region::RegionLimits::default(),
    };
    let source = load_source_file_with_limits(&flags.path, region_limits)?;
    let new = &source.definition;
    let compiled = crate::declarative::compile::compile(new.clone()).map_err(|e| e.join("; "))?;
    let lints = crate::declarative::structural_lints(new, &compiled);
    let corpus_present = corpus_file_present(&flags.path, flags.corpus.as_deref());
    let corpus = if flags.auto_seed {
        crate::declarative::auto_seed_corpus(new)
    } else {
        load_corpus(&flags.path, flags.corpus.as_deref(), new)?
    };
    let profile = load_check_profile(flags.profile.as_deref(), new)?;
    if flags.plan {
        return emit_check_plan(profile.as_ref(), new, &corpus, flags.json);
    }
    let old = flags
        .prior
        .as_deref()
        .map(|p| load_definition_file(p, region_limits))
        .transpose()?;
    let baseline = load_baseline(flags.baseline.as_deref(), &new.machine_id)?;
    let bounds = profile
        .as_ref()
        .map(|item| item.limits)
        .unwrap_or(crate::declarative::Bounds {
            max_nodes: crate::declarative::profile::DEFAULT_MAX_NODES,
            max_depth: crate::declarative::profile::DEFAULT_MAX_DEPTH,
        });
    let mut boundary_exprs = Vec::new();
    for raw in &flags.bounds_exprs {
        let ast = crate::declarative::def::validate_state_expr(
            new,
            raw,
            &format!("--bound `{raw}`"),
        )?;
        boundary_exprs.push((raw.clone(), ast));
    }
    if let Some(ref p) = profile {
        if let Some(ref raw) = p.boundary {
            let ast = crate::declarative::def::validate_state_expr(
                new,
                raw,
                "profile limits.boundary",
            )?;
            boundary_exprs.push((raw.clone(), ast));
        }
    }
    let explore_options = crate::declarative::explore::ExploreOptions {
        boundary: boundary_exprs.clone(),
        fail_fast: flags.fail_fast,
        threads: flags.threads,
    };
    let mut result = crate::declarative::region::check_source_with_options(
        &source,
        old.as_ref(),
        &corpus,
        &baseline,
        bounds,
        &explore_options,
    )?;
    // When no explicit corpus was supplied, run a fast bounded auto-seeded falsifier
    // strictly to catch real runtime invariant violations early. Never upgrades REFUSED to ACCEPT.
    let mut falsified_by_autoseed = false;
    if !flags.auto_seed && !corpus_present && result.invariant_refusals.is_empty() && !new.invariants.is_empty() {
        let seed_corpus = crate::declarative::auto_seed_corpus(new);
        let falsify_bounds = crate::declarative::Bounds {
            max_nodes: 256.min(bounds.max_nodes),
            max_depth: 12.min(bounds.max_depth),
        };
        let falsify_opts = crate::declarative::explore::ExploreOptions {
            boundary: boundary_exprs.clone(),
            fail_fast: flags.fail_fast,
            threads: flags.threads,
        };
        let (_, found) = crate::declarative::explore::explore_with_options(
            &compiled,
            &seed_corpus,
            falsify_bounds,
            &falsify_opts,
            true,
        );
        if !found.is_empty() {
            falsified_by_autoseed = true;
            result.verdict = "REFUSED".to_string();
            result.flat.verdict = "REFUSED".to_string();
            result.flat.judgment.verdict = "refused".to_string();
            result.flat.judgment.status = "witnessed_violation".to_string();
            for witness in &found {
                let rule = witness.diagnostic.rule.as_deref().unwrap_or("unknown");
                result.flat.judgment.reasons.push(format!(
                    "auto-seeded falsifier found model transition violating invariant {rule}; runtime blocked the proposal"
                ));
                let steps: Vec<String> = witness
                    .steps
                    .iter()
                    .map(|s| match &s.case {
                        Some(c) => format!("{}[{c}]", s.command),
                        None => s.command.clone(),
                    })
                    .collect();
                result.text.push_str(&format!(
                    "\nINVARIANT-REFUSAL WITNESS (auto-seeded bounded falsifier)\n  {} rule={rule}\n  steps: {}\n",
                    witness.diagnostic.code.as_str(),
                    steps.join(" -> ")
                ));
            }
            result.invariant_refusals = found;
        }
    }

    // Track which corpus was used for replay & simulation (auto-seed when user invokes --simulate without corpus).
    let sim_corpus = if corpus.commands.is_empty() && !flags.auto_seed && !flags.release {
        crate::declarative::auto_seed_corpus(new)
    } else {
        corpus.clone()
    };
    let should_simulate = !flags.release
        && (flags.simulate
            || (!result.exploration_complete
                && result.invariant_refusals.is_empty()
                && (!new.invariants.is_empty()
                    || result.exploration.reach_witnesses.len() < new.reach.len())));
    let mut sim_result: Option<crate::declarative::sim_check::SimResult> = None;
    if should_simulate {
        let sim_opts = crate::declarative::sim_check::SimOptions {
            seed: flags
                .sim_seed
                .unwrap_or(crate::declarative::sim_check::DEFAULT_SIM_SEED),
            max_steps: flags
                .sim_steps
                .unwrap_or(crate::declarative::sim_check::DEFAULT_SIM_STEPS),
            traces: flags
                .sim_traces
                .unwrap_or(crate::declarative::sim_check::DEFAULT_SIM_TRACES),
            fail_fast: flags.fail_fast,
            boundary: boundary_exprs.clone(),
        };
        let sim = crate::declarative::sim_check::run_simulation(&compiled, &sim_corpus, &sim_opts);
        for (name, steps) in &sim.reach_witnesses {
            result
                .exploration
                .reach_witnesses
                .entry(name.clone())
                .or_insert_with(|| steps.clone());
        }
        if !sim.invariant_refusals.is_empty() {
            for w in &sim.invariant_refusals {
                let rule = w.diagnostic.rule.as_deref().unwrap_or("unknown");
                if !result
                    .invariant_refusals
                    .iter()
                    .any(|existing| existing.diagnostic.rule.as_deref() == Some(rule))
                {
                    result.invariant_refusals.push(w.clone());
                    result.flat.judgment.reasons.push(format!(
                        "seeded simulation (seed={}) found deep invariant violation {rule} at depth {}",
                        sim.seed,
                        w.steps.len()
                    ));
                }
            }
            result.verdict = "REFUSED".to_string();
            result.flat.verdict = "REFUSED".to_string();
            result.flat.judgment.verdict = "refused".to_string();
            result.flat.judgment.status = "witnessed_violation".to_string();
        }
        if flags.simulate && !new.reach.is_empty() && result.flat.judgment.status == "unreached_property" {
            let all_reached = new
                .reach
                .keys()
                .all(|k| result.exploration.reach_witnesses.contains_key(k));
            if all_reached {
                result.flat.judgment.reasons.retain(|r| !r.starts_with("reach property `"));
                if result.flat.judgment.reasons.is_empty() && result.flat.report.obligations.uncovered == 0 {
                    result.verdict = "ACCEPT".to_string();
                    result.flat.verdict = "ACCEPT".to_string();
                    result.flat.judgment.verdict = "accept".to_string();
                    result.flat.judgment.status = "accepted".to_string();
                }
            }
        }
        sim_result = Some(sim);
    }

    let mut envelope = profile
        .as_ref()
        .map(|item| crate::declarative::profile::evaluate(item, new, &corpus, &result.exploration));
    attach_profile_baseline(
        envelope.as_mut(),
        profile.as_ref(),
        old.as_ref(),
        &result,
        &corpus,
    )?;
    if let Some(ref emit_path) = flags.emit_scenario {
        if let Some(first_violation) = result.invariant_refusals.first() {
            let trace_corpus = if falsified_by_autoseed || sim_result.is_some() {
                &sim_corpus
            } else {
                &corpus
            };
            let steps = crate::declarative::sim_check::replay_path_steps(
                &compiled,
                trace_corpus,
                &first_violation.steps,
            );
            write_emitted_scenario(emit_path, &flags.path, &new.machine_id, &steps)?;
        }
    }
    let trace_corpus = if falsified_by_autoseed || sim_result.is_some() {
        &sim_corpus
    } else {
        &corpus
    };
    let hint = if flags.auto_seed {
        None
    } else {
        corpus_hint(
            &flags.path,
            flags.corpus.as_deref(),
            result.verdict == "REFUSED",
        )
    };
    emit_check(
        &flags,
        new,
        &compiled,
        trace_corpus,
        &result,
        &lints,
        falsified_by_autoseed,
        sim_result.as_ref(),
        envelope.as_ref(),
        hint.as_deref(),
    )?;
    let release_pass = result.flat.release.release.verdict == "release_pass";
    let accepted = result.verdict == "ACCEPT" && (!flags.release || release_pass);
    let goals_ok = envelope
        .as_ref()
        .map(crate::declarative::profile::required_goals_satisfied)
        .unwrap_or(true);
    Ok(if accepted && goals_ok { 0 } else { 1 })
}

fn load_check_profile(
    path: Option<&Path>,
    definition: &crate::declarative::Definition,
) -> Result<Option<crate::declarative::profile::Profile>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let profile = crate::declarative::profile::load_profile(path)?;
    crate::declarative::profile::bind_profile(&profile, definition)?;
    Ok(Some(profile))
}

fn emit_check_plan(
    profile: Option<&crate::declarative::profile::Profile>,
    definition: &crate::declarative::Definition,
    corpus: &crate::declarative::CaseCorpus,
    json_output: bool,
) -> Result<i32, String> {
    let profile = profile.ok_or("--plan requires --profile")?;
    let plan = crate::declarative::profile::plan(profile, definition, corpus);
    if json_output {
        println!("{}", canonical_json(&plan));
    } else {
        print!("{}", crate::declarative::profile::render_plan(&plan));
    }
    Ok(0)
}

fn emit_corpus_template(machine: &Path) -> Result<i32, String> {
    let source = load_source_file(machine)?;
    let body = crate::declarative::starter_corpus(&source.definition);
    println!(
        "{}",
        serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?
    );
    Ok(0)
}

fn corpus_hint(machine: &Path, explicit: Option<&Path>, refused: bool) -> Option<String> {
    if !refused || corpus_file_present(machine, explicit) {
        return None;
    }
    Some(format!(
        "ik corpus init {} --out {}",
        machine.display(),
        machine.with_extension("cases.json").display()
    ))
}

fn corpus_file_present(machine: &Path, explicit: Option<&Path>) -> bool {
    explicit.is_some() || machine.with_extension("cases.json").is_file()
}

/// Adds the optional `profile` and `next_actions` members to a check JSON object.
fn attach_check_extras(
    value: &mut Value,
    envelope: Option<&crate::declarative::profile::ProfileEnvelope>,
    hint: Option<&str>,
) -> Result<(), String> {
    let profile = match envelope {
        Some(envelope) => Some(serde_json::to_value(envelope).map_err(|err| err.to_string())?),
        None => None,
    };
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    if let Some(profile) = profile {
        object.insert("profile".to_string(), profile);
    }
    if let Some(hint) = hint {
        object.insert("next_actions".to_string(), json!([hint]));
    }
    Ok(())
}

fn relative_path_from_dir(from_dir: &Path, target: &Path) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let from_abs = std::fs::canonicalize(from_dir).unwrap_or_else(|_| cwd.join(from_dir));
    let target_abs = std::fs::canonicalize(target).unwrap_or_else(|_| cwd.join(target));
    let from_comps: Vec<std::path::Component<'_>> = from_abs.components().collect();
    let target_comps: Vec<std::path::Component<'_>> = target_abs.components().collect();
    let common = from_comps
        .iter()
        .zip(target_comps.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = Vec::new();
    for comp in &from_comps[common..] {
        if matches!(comp, std::path::Component::Normal(_)) {
            parts.push("..".to_string());
        }
    }
    for comp in &target_comps[common..] {
        match comp {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => parts.push("..".to_string()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Ok(target.display().to_string());
            }
        }
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

fn write_emitted_scenario(
    out_path: &Path,
    machine_path: &Path,
    machine_id: &str,
    steps: &[crate::declarative::sim_check::TraceStep],
) -> Result<(), String> {
    let mut out_steps = Vec::with_capacity(steps.len());
    for (idx, step) in steps.iter().enumerate() {
        if step.outcome == "record_effect" {
            return Err(
                "--emit-scenario cannot automatically synthesize external effect IDs for `record_effect` steps"
                    .to_string(),
            );
        }
        let expect = if step.outcome == "refused" {
            format!(
                "refused:{}",
                step.code.as_deref().unwrap_or("IK_INVARIANT_FAILED")
            )
        } else {
            step.outcome.clone()
        };
        out_steps.push(json!({
            "kind": "command",
            "label": format!("{}-{}", step.command, idx + 1),
            "name": step.command,
            "args": step.args,
            "expect": expect,
        }));
    }
    let out_dir = match out_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
            parent
        }
        _ => Path::new("."),
    };
    let rel_machine = relative_path_from_dir(out_dir, machine_path)?;
    let doc = json!({
        "scenario_version": 1,
        "machine": rel_machine,
        "aggregate": format!("{machine_id}-repro-1"),
        "steps": out_steps,
    });
    let pretty = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(out_path, format!("{pretty}\n"))
        .map_err(|e| format!("write {}: {e}", out_path.display()))
}

fn format_actionable_trace(steps: &[crate::declarative::sim_check::TraceStep]) -> String {
    let mut out = String::new();
    for step in steps {
        let case_tag = step
            .case
            .as_ref()
            .map(|c| format!("[{c}]"))
            .unwrap_or_default();
        let args_str = step
            .args
            .as_object()
            .filter(|m| !m.is_empty())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let diff_str = if step.fact_diff.is_empty() {
            String::new()
        } else {
            let items = step
                .fact_diff
                .iter()
                .map(|d| match &d.before {
                    Some(b) => format!("{}: {} -> {}", d.fact, b, d.after),
                    None => format!("{}={}", d.fact, d.after),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(" {{{items}}}")
        };
        if step.outcome == "refused" {
            let code = step.code.as_deref().unwrap_or("REFUSED");
            let msg = step.message.as_deref().unwrap_or("");
            out.push_str(&format!(
                "    {}. [{}] --{}{case_tag}{args_str}--> REFUSED ({code}: {msg})\n",
                step.index + 1,
                step.from_state,
                step.command
            ));
        } else {
            out.push_str(&format!(
                "    {}. [{}] --{}{case_tag}{args_str}--> [{}]{diff_str}\n",
                step.index + 1,
                step.from_state,
                step.command,
                step.to_state
            ));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_check(
    flags: &CheckFlags,
    new: &crate::declarative::Definition,
    compiled: &crate::declarative::CompiledMachine,
    corpus: &crate::declarative::CaseCorpus,
    result: &crate::declarative::region::RegionCheckResult,
    lints: &[crate::declarative::StructuralLint],
    falsified_by_autoseed: bool,
    sim_result: Option<&crate::declarative::sim_check::SimResult>,
    envelope: Option<&crate::declarative::profile::ProfileEnvelope>,
    hint: Option<&str>,
) -> Result<(), String> {
    let reach_results: Vec<Value> = new
        .reach
        .iter()
        .map(|(name, expr)| {
            let steps = result.exploration.reach_witnesses.get(name);
            json!({
                "name": name,
                "expr": expr,
                "reached": steps.is_some(),
                "steps": steps,
            })
        })
        .collect();
    if flags.compact {
        let mut value = json!({
            "verdict": result.verdict,
            "status": result.flat.judgment.status,
            "assurance_mode": if flags.auto_seed { "auto_seeded_bounded" } else { "corpus_bounded" },
            "release_verdict": result.flat.release.release.verdict,
            "machine_id": result.flat.report.machine_id,
            "spec_hash": result.flat.report.spec_hash,
            "obligations": {
                "total": result.flat.report.obligations.total,
                "satisfied": result.flat.report.obligations.satisfied,
                "uncovered": result.flat.report.obligations.uncovered,
                "falsified": result.flat.report.obligations.falsified,
                "incomplete": result.flat.report.obligations.incomplete,
                "uncovered_ids": result.flat.report.obligations.uncovered_ids,
            },
            "reasons": result.flat.judgment.reasons,
            "structural_lints": lints,
            "invariant_witnesses": crate::declarative::region::invariant_witness_json(&result.invariant_refusals),
        });
        if !reach_results.is_empty() {
            value.as_object_mut().unwrap().insert("reach_results".into(), json!(reach_results));
        }
        if let Some(pruned) = result.exploration.boundary_pruned {
            value.as_object_mut().unwrap().insert("boundary_pruned".into(), json!(pruned));
        }
        if let Some(sim) = sim_result {
            value.as_object_mut().unwrap().insert("simulation".into(), json!({
                "seed": sim.seed,
                "traces_run": sim.traces_run,
                "steps_executed": sim.steps_executed,
                "max_depth_reached": sim.max_depth_reached,
                "boundary_pruned": sim.boundary_pruned,
            }));
        }
        attach_check_extras(&mut value, envelope, hint)?;
        println!("{}", canonical_json(&value));
    } else if flags.json {
        let mut value = json!({
            "verdict": result.verdict,
            "judgment": result.flat.judgment,
            "release": result.flat.release,
            "report": result.flat.report,
            "invariant_witnesses": crate::declarative::region::invariant_witness_json(&result.invariant_refusals),
        });
        if flags.auto_seed {
            if let Some(object) = value.as_object_mut() {
                object.insert("assurance_mode".into(), json!("auto_seeded_bounded"));
            }
        }
        if !reach_results.is_empty() {
            value.as_object_mut().unwrap().insert("reach_results".into(), json!(reach_results));
        }
        if let Some(pruned) = result.exploration.boundary_pruned {
            value.as_object_mut().unwrap().insert("boundary_pruned".into(), json!(pruned));
        }
        if let Some(sim) = sim_result {
            value.as_object_mut().unwrap().insert("simulation".into(), json!({
                "seed": sim.seed,
                "traces_run": sim.traces_run,
                "steps_executed": sim.steps_executed,
                "max_depth_reached": sim.max_depth_reached,
                "boundary_pruned": sim.boundary_pruned,
            }));
        }
        if !lints.is_empty() {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "structural_lints".into(),
                    serde_json::to_value(lints).map_err(|err| err.to_string())?,
                );
            }
        }
        attach_check_extras(&mut value, envelope, hint)?;
        if let Some(definition) = &result.flat.definition {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "definition".into(),
                    serde_json::to_value(definition).map_err(|err| err.to_string())?,
                );
            }
        }
        println!("{}", canonical_json(&value));
    } else if flags.summary {
        let cov = &result.exploration.coverage;
        let mode = if flags.auto_seed {
            "auto_seeded_bounded (advisory; supply --corpus for --release)"
        } else {
            "corpus_bounded (not exhaustive proof)"
        };
        println!(
            "{}  {} · states {}/{} · commands {}/{} · outcomes {}/{} · terminals {}/{}",
            result.verdict,
            result.flat.report.machine_id,
            cov.executably_reached_states.len(),
            cov.declared_states.len(),
            cov.executed_commands.len(),
            cov.declared_commands.len(),
            cov.executed_outcomes.len(),
            cov.declared_outcomes.len(),
            cov.reached_terminal_states.len(),
            cov.terminal_states.len(),
        );
        println!("mode: {mode}");
        if let Some(pruned) = result.exploration.boundary_pruned {
            println!(
                "boundary: complete={} (nodes={}, pruned={pruned})",
                result.exploration.complete, result.exploration.nodes
            );
        }
        if let Some(sim) = sim_result {
            println!(
                "simulation: seed={} · traces={} · max_depth={} · steps={}",
                sim.seed, sim.traces_run, sim.max_depth_reached, sim.steps_executed
            );
        }
        if !new.reach.is_empty() {
            println!("reach ({}):", new.reach.len());
            for (name, expr) in &new.reach {
                match result.exploration.reach_witnesses.get(name) {
                    Some(steps) => println!("  [ok] {name} ({} steps): {expr}", steps.len()),
                    None => println!("  [MISS] {name} (not witnessed): {expr}"),
                }
            }
        }
        if !lints.is_empty() {
            print!("{}", crate::declarative::render_lints(lints));
        }
        if !result.invariant_refusals.is_empty() {
            println!("invariant violations witnessed ({}):", result.invariant_refusals.len());
            for w in &result.invariant_refusals {
                let rule = w.diagnostic.rule.as_deref().unwrap_or("unknown");
                let steps = crate::declarative::sim_check::replay_path_steps(
                    compiled,
                    corpus,
                    &w.steps,
                );
                println!("  - {rule} ({} steps):", steps.len());
                print!("{}", format_actionable_trace(&steps));
            }
            println!(
                "reproduce: ik simulate {} --check --open",
                flags.path.display()
            );
        }
        if result.verdict != "ACCEPT" && !result.flat.judgment.reasons.is_empty() {
            for r in &result.flat.judgment.reasons {
                println!("reason: {r}");
            }
        }
        if let Some(hint) = hint {
            println!("next: {hint}");
            println!("   or: ik check {} --auto-seed --summary", flags.path.display());
        }
    } else {
        if flags.auto_seed {
            println!("AUTO-SEEDED (advisory bounded check — supply --corpus for --release)\n");
        }
        print!("{}", result.text);
        if let Some(pruned) = result.exploration.boundary_pruned {
            println!(
                "\nBOUNDARY\n  complete: {}\n  nodes: {}\n  pruned: {pruned}",
                result.exploration.complete, result.exploration.nodes
            );
        }
        if let Some(sim) = sim_result {
            println!(
                "\nSIMULATION\n  seed: {}\n  traces: {}\n  max_depth: {}\n  steps: {}",
                sim.seed, sim.traces_run, sim.max_depth_reached, sim.steps_executed
            );
        }
        if !new.reach.is_empty() {
            println!("\nREACH PROPERTIES");
            for (name, expr) in &new.reach {
                match result.exploration.reach_witnesses.get(name) {
                    Some(steps) => println!("  [ok] {name} ({} steps): {expr}", steps.len()),
                    None => println!("  [MISS] {name} (not witnessed): {expr}"),
                }
            }
        }
        if !result.invariant_refusals.is_empty() {
            println!("\nACTIONABLE COUNTEREXAMPLE TRACES");
            for w in &result.invariant_refusals {
                let rule = w.diagnostic.rule.as_deref().unwrap_or("unknown");
                let steps = crate::declarative::sim_check::replay_path_steps(
                    compiled,
                    corpus,
                    &w.steps,
                );
                println!("  rule={rule} ({} steps):", steps.len());
                print!("{}", format_actionable_trace(&steps));
            }
            println!(
                "  reproduce interactively: ik simulate {} --check --open",
                flags.path.display()
            );
        }
        if !lints.is_empty() {
            print!("{}", crate::declarative::render_lints(lints));
        }
        if let Some(envelope) = envelope {
            print!("{}", crate::declarative::profile::render_profile(envelope));
        }
        if let Some(hint) = hint {
            println!("NEXT COMMAND\n  {hint}");
            if !falsified_by_autoseed {
                println!("  (or try an instant advisory check: ik check {} --auto-seed --summary)", flags.path.display());
            }
        }
    }
    Ok(())
}

fn run_corpus(args: &[String]) -> i32 {
    match run_corpus_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik corpus: {msg}");
            eprintln!("usage: ik corpus init <machine.ik.toml> --out <cases.json> [--smart]");
            2
        }
    }
}

fn run_corpus_inner(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: ik corpus init <machine.ik.toml> --out <cases.json> [--smart]");
        return Ok(0);
    }
    if args.first().map(String::as_str) != Some("init") {
        return Err("corpus command is required".into());
    }
    let mut machine = None;
    let mut out = None;
    let mut smart = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(path_value(args, i, "--out")?);
            }
            "--smart" => smart = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            name if machine.is_none() => machine = Some(PathBuf::from(name)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    let machine = machine.ok_or("machine definition path is required")?;
    let out = out.ok_or("--out requires a path")?;
    let source = load_source_file(&machine)?;
    let body = if smart {
        crate::declarative::smart_starter_corpus(&source.definition)
    } else {
        crate::declarative::starter_corpus(&source.definition)
    };
    let text = serde_json::to_string_pretty(&body).map_err(|err| err.to_string())?;
    std::fs::write(&out, format!("{text}\n"))
        .map_err(|err| format!("write {}: {err}", out.display()))?;
    Ok(0)
}

fn load_corpus(
    machine: &Path,
    explicit: Option<&Path>,
    def: &crate::declarative::Definition,
) -> Result<crate::declarative::CaseCorpus, String> {
    crate::declarative::corpus::load_corpus(machine, explicit, def)
}

fn load_baseline(explicit: Option<&Path>, machine_id: &str) -> Result<Vec<String>, String> {
    let default = PathBuf::from("validation/verification_baseline.json");
    let path = explicit.or_else(|| default.exists().then_some(default.as_path()));
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let file: BaselineFile =
        serde_json::from_str(&text).map_err(|e| format!("baseline {}: {e}", path.display()))?;
    if file.machine_id != machine_id {
        return Ok(Vec::new());
    }
    Ok(file.uncovered_mandatory)
}

#[derive(serde::Deserialize)]
struct BaselineFile {
    machine_id: String,
    uncovered_mandatory: Vec<String>,
}

fn attach_profile_baseline(
    envelope: Option<&mut crate::declarative::profile::ProfileEnvelope>,
    profile: Option<&crate::declarative::profile::Profile>,
    prior: Option<&crate::declarative::Definition>,
    result: &crate::declarative::region::RegionCheckResult,
    corpus: &crate::declarative::CaseCorpus,
) -> Result<(), String> {
    let (Some(envelope), Some(profile), Some(prior)) = (envelope, profile, prior) else {
        return Ok(());
    };
    let prior_run = result
        .prior_exploration
        .as_ref()
        .ok_or("prior exploration was not produced")?;
    let baseline =
        crate::declarative::profile::baseline_against(profile, envelope, prior, prior_run, corpus);
    envelope.baseline = Some(baseline);
    Ok(())
}

fn run_profile(args: &[String]) -> i32 {
    match run_profile_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik profile: {msg}");
            eprintln!("usage: ik profile diff <old.toml> <new.toml> [--json]");
            2
        }
    }
}

fn run_profile_inner(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: ik profile diff <old.toml> <new.toml> [--json]");
        return Ok(0);
    }
    if args.is_empty() {
        return Err("profile command is required".into());
    }
    if args.first().map(String::as_str) != Some("diff") {
        return Err(format!("unknown profile command `{}`", args[0]));
    }
    let (old, new, json_output) = parse_profile_diff(&args[1..])?;
    let old = crate::declarative::profile::load_profile(&old)?;
    let new = crate::declarative::profile::load_profile(&new)?;
    let diff = crate::declarative::profile::diff_profiles(&old, &new);
    if json_output {
        println!("{}", canonical_json(&diff));
    } else {
        print!(
            "{}",
            crate::declarative::profile::render_profile_diff(&diff)
        );
    }
    Ok(0)
}

fn parse_profile_diff(args: &[String]) -> Result<(PathBuf, PathBuf, bool), String> {
    let mut paths = Vec::new();
    let mut json_output = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json_output = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    if paths.len() != 2 {
        return Err("diff requires <old.toml> <new.toml>".into());
    }
    Ok((paths.remove(0), paths.remove(0), json_output))
}

fn run_evidence(args: &[String]) -> i32 {
    match run_evidence_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik evidence: {msg}");
            eprintln!("usage: ik evidence verify <evidence.json> --machine <m.ik.toml> [--profile <p.toml>] [--corpus <c.json>] [--runtime-binary <path>] [--json]");
            2
        }
    }
}

fn run_evidence_inner(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: ik evidence verify <evidence.json> --machine <m.ik.toml> [--profile <p.toml>] [--corpus <c.json>] [--runtime-binary <path>] [--json]");
        return Ok(0);
    }
    if args.is_empty() {
        return Err("evidence command is required".into());
    }
    if args.first().map(String::as_str) != Some("verify") {
        return Err(format!("unknown evidence command `{}`", args[0]));
    }
    let flags = parse_evidence_flags(&args[1..])?;
    let evidence = read_evidence(&flags.evidence)?;
    let current = current_view(&flags)?;
    let report = crate::evidence_verify::verify(&evidence, &current)?;
    emit_evidence(&report, flags.json);
    Ok(if report.overall == "current" { 0 } else { 1 })
}

struct EvidenceFlags {
    evidence: PathBuf,
    machine: PathBuf,
    profile: Option<PathBuf>,
    corpus: Option<PathBuf>,
    runtime_binary: Option<PathBuf>,
    json: bool,
}

fn parse_evidence_flags(args: &[String]) -> Result<EvidenceFlags, String> {
    let mut evidence = None;
    let mut values: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut json_output = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json_output = true,
            flag @ ("--machine" | "--profile" | "--corpus" | "--runtime-binary") => {
                i += 1;
                values.insert(flag, path_value(args, i, flag)?);
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path if evidence.is_none() => evidence = Some(PathBuf::from(path)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    Ok(EvidenceFlags {
        evidence: evidence.ok_or("evidence path is required")?,
        machine: values.remove("--machine").ok_or("--machine is required")?,
        profile: values.remove("--profile"),
        corpus: values.remove("--corpus"),
        runtime_binary: values.remove("--runtime-binary"),
        json: json_output,
    })
}

fn read_evidence(path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("evidence {}: {err}", path.display()))
}

fn current_view(flags: &EvidenceFlags) -> Result<crate::evidence_verify::CurrentView, String> {
    let machine = load_def(&flags.machine).map_err(|errors| errors.join("; "))?;
    let identity = machine.identity();
    let definition = load_definition(&flags.machine).map_err(|errors| errors.join("; "))?;
    let profile = flags
        .profile
        .as_deref()
        .map(crate::declarative::profile::load_profile)
        .transpose()?;
    let identity_profile = profile
        .as_ref()
        .map(crate::declarative::profile::profile_identity);
    let profile_digest = identity_profile.as_ref().map(|item| item.digest.clone());
    let goal_digests = identity_profile.map(|item| item.goals).unwrap_or_default();
    let corpus = load_corpus(&flags.machine, flags.corpus.as_deref(), &definition)?;
    Ok(crate::evidence_verify::CurrentView {
        machine_id: identity.machine_id.clone(),
        machine_version: identity.version.0,
        spec_hash: identity.spec_hash.0.clone(),
        profile_digest,
        goal_digests,
        corpus_digest: Some(corpus.digest()),
        scenario_digest: sibling_scenario_digest(&flags.evidence)?,
        runtime_sha256: flags
            .runtime_binary
            .as_deref()
            .map(file_sha256)
            .transpose()?,
    })
}

fn sibling_scenario_digest(evidence: &Path) -> Result<Option<String>, String> {
    let Some(parent) = evidence.parent() else {
        return Ok(None);
    };
    let path = parent.join("scenario.json");
    if !path.is_file() {
        return Ok(None);
    }
    file_sha256(&path).map(Some)
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    Ok(crate::canonical::sha256_hex(&bytes))
}

fn emit_evidence(report: &crate::evidence_verify::VerifyReport, json_output: bool) {
    if json_output {
        println!("{}", canonical_json(report));
    } else {
        print!("{}", crate::evidence_verify::render_text(report));
    }
}
