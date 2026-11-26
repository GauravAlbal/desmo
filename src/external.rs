//! Versioned external Python reducer adapter.
//! The adapter has no storage surface; Rust remains the transaction authority.

use crate::canonical::{canonical_json, digest_json, normalize_shared_numbers, sha256_hex};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use crate::identity::{MachineIdentity, MachineSpecHash, MachineVersion};
use crate::machine::{ApplyOutcome, Decision, DomainEvent, Machine};
use crate::protocol::{ProtocolVersion, PROTOCOL_MAJOR, PROTOCOL_MINOR};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[cfg(unix)]
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DESCRIPTOR: &str = "invariant-kernel.python-reducer/1";
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 300_000;
const TERMINATION_GRACE: Duration = Duration::from_millis(50);
const PROCESS_TOKEN_ENV: &str = "IK_REDUCER_PROCESS_TOKEN";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReducerIdentity {
    pub reducer_name: String,
    pub reducer_version: u64,
    pub code_digest: String,
    pub command_schema_digest: String,
    pub state_schema_digest: String,
}

/// Direct argv plus the flat, hash-bound reducer descriptor. Launch routing and
/// timeout are deliberately excluded from the semantic descriptor hash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerSpec {
    pub schema: String,
    pub entrypoint: String,
    pub reducer_name: String,
    pub reducer_version: u64,
    pub code_digest: String,
    pub command_schema_digest: String,
    pub state_schema_digest: String,
    pub initial_state_digest: String,
    pub initial_state: Value,
    pub command: Vec<String>,
    pub timeout_ms: u64,
    /// Optional kernel-enforced post-state laws. Not part of the binding hash.
    #[serde(default)]
    pub kernel_laws: Vec<KernelLawSpec>,
    /// Optional source file hashed and compared to `code_digest`. Not part of the binding hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_source: Option<String>,
}

/// Constraint on acceptable reducer results. Not a second transition model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelLawSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(rename = "assert")]
    pub assertion: String,
}

struct CompiledKernelLaw {
    name: String,
    when_src: Option<String>,
    when: Option<crate::declarative::expr::Expr>,
    assertion_src: String,
    assertion: crate::declarative::expr::Expr,
}

#[derive(Clone, Debug, Serialize)]
struct BindingDescriptor<'a> {
    schema: &'a str,
    entrypoint: &'a str,
    reducer_name: &'a str,
    reducer_version: u64,
    code_digest: &'a str,
    command_schema_digest: &'a str,
    state_schema_digest: &'a str,
    initial_state_digest: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kernel_laws_digest: Option<&'a str>,
}

impl WorkerSpec {
    pub fn from_json(text: &str) -> Result<Self, Diagnostic> {
        serde_json::from_str(text).map_err(|error| {
            Diagnostic::new(
                DiagnosticCode::IkProtocolInvalid,
                Some("reducer.spec"),
                format!("invalid reducer worker spec: {error}"),
            )
        })
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        if self.schema != DESCRIPTOR {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProtocolVersionUnknown,
                Some("reducer.schema"),
                format!("unsupported reducer descriptor `{}`", self.schema),
            ));
        }
        if self.entrypoint.is_empty() || self.entrypoint.contains('\0') {
            return Err(protocol_error("reducer entrypoint must be non-empty"));
        }
        if self.reducer_name.is_empty() {
            return Err(protocol_error("reducer_name must be non-empty"));
        }
        if self.reducer_version == 0 {
            return Err(protocol_error("reducer_version must be at least 1"));
        }
        if self.command.is_empty()
            || self
                .command
                .iter()
                .any(|argument| argument.is_empty() || argument.contains('\0'))
        {
            return Err(protocol_error(
                "reducer command must contain non-empty argv entries",
            ));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(protocol_error(format!(
                "timeout_ms must be between 1 and {MAX_TIMEOUT_MS}",
            )));
        }
        for (name, digest) in [
            ("code_digest", &self.code_digest),
            ("command_schema_digest", &self.command_schema_digest),
            ("state_schema_digest", &self.state_schema_digest),
            ("initial_state_digest", &self.initial_state_digest),
        ] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkProtocolInvalid,
                    Some(format!("reducer.{name}")),
                    format!("{name} must be exactly 64 lowercase hexadecimal characters"),
                ));
            }
        }
        if sha256_hex(canonical_json(&self.initial_state).as_bytes()) != self.initial_state_digest {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineHashMismatch,
                Some("reducer.initial_state_digest"),
                "worker spec initial state does not match binding",
            ));
        }
        Ok(())
    }

    fn descriptor<'a>(&'a self, kernel_laws_digest: Option<&'a str>) -> BindingDescriptor<'a> {
        BindingDescriptor {
            schema: &self.schema,
            entrypoint: &self.entrypoint,
            reducer_name: &self.reducer_name,
            reducer_version: self.reducer_version,
            code_digest: &self.code_digest,
            command_schema_digest: &self.command_schema_digest,
            state_schema_digest: &self.state_schema_digest,
            initial_state_digest: &self.initial_state_digest,
            kernel_laws_digest,
        }
    }

    fn machine_identity(&self) -> MachineIdentity {
        let laws_digest = (!self.kernel_laws.is_empty()).then(|| digest_json(&self.kernel_laws));
        MachineIdentity {
            machine_id: self.reducer_name.clone(),
            version: MachineVersion(self.reducer_version),
            spec_hash: MachineSpecHash(digest_json(&self.descriptor(laws_digest.as_deref()))),
        }
    }

    fn reducer_identity(&self) -> ReducerIdentity {
        ReducerIdentity {
            reducer_name: self.reducer_name.clone(),
            reducer_version: self.reducer_version,
            code_digest: self.code_digest.clone(),
            command_schema_digest: self.command_schema_digest.clone(),
            state_schema_digest: self.state_schema_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct WorkerRequest<'a> {
    protocol: ProtocolVersion,
    op: &'a str,
    entrypoint: &'a str,
    identity: &'a ReducerIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effect: Option<&'a EffectStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<&'a DomainEvent>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResponse {
    protocol: ProtocolVersion,
    ok: bool,
    entrypoint: String,
    reducer_identity: ReducerIdentity,
    result: Option<Value>,
    diagnostic: Option<WireDiagnostic>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionResult {
    decision: String,
    #[serde(default)]
    event: Option<DomainEvent>,
    #[serde(default)]
    note: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyResult {
    state: Value,
    terminal: bool,
    #[serde(default)]
    effect: Option<EffectIntentV1>,
    #[serde(default)]
    outcome: Option<EffectOutcomeV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDiagnostic {
    code: String,
    rule: Option<String>,
    message: String,
}

/// Recycle a long-lived worker after this many calls to bound reducer-side memory growth.
const DEFAULT_RECYCLE_AFTER: u64 = 5_000;

fn recycle_after() -> u64 {
    std::env::var("IK_RECYCLE_AFTER")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_RECYCLE_AFTER)
}

pub struct ExternalMachine {
    spec: WorkerSpec,
    identity: MachineIdentity,
    initial_state: Value,
    timeout: Duration,
    worker: Mutex<Option<Worker>>,
    /// `IK_REDUCER_PARANOID=1`: replay every call on a fresh worker and fail closed on any
    /// divergence. Detects reducers that violate the purity contract; default off.
    paranoid: bool,
    laws: Vec<CompiledKernelLaw>,
}

impl ExternalMachine {
    pub fn from_json(text: &str) -> Result<Self, Diagnostic> {
        Self::from_spec(WorkerSpec::from_json(text)?)
    }

    pub fn from_spec(spec: WorkerSpec) -> Result<Self, Diagnostic> {
        spec.validate()?;
        verify_code_source(&spec)?;
        let laws = compile_kernel_laws(&spec.kernel_laws)?;
        enforce_kernel_laws(&laws, &spec.initial_state)?;
        let timeout = Duration::from_millis(spec.timeout_ms);
        let identity = spec.machine_identity();
        let initial_state = spec.initial_state.clone();
        Ok(Self {
            spec,
            identity,
            initial_state,
            timeout,
            worker: Mutex::new(None),
            paranoid: std::env::var("IK_REDUCER_PARANOID").as_deref() == Ok("1"),
            laws,
        })
    }

    /// One JSON-RPC 2.0 call on the long-lived worker. Any transport or protocol failure
    /// discards the worker (fail closed); a domain refusal is an ordinary `result` and does not.
    fn call(&self, method: &'static str, params: Value) -> Result<Value, Diagnostic> {
        let witness = self.paranoid.then(|| params.clone());
        let mut guard = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|w| w.calls >= recycle_after()) {
            *guard = None;
        }
        if guard.is_none() {
            *guard = Some(Worker::spawn(&self.spec.command)?);
        }
        let outcome =
            guard
                .as_mut()
                .expect("worker was just ensured")
                .call(method, params, self.timeout);
        if let (Ok(value), Some(witness)) = (&outcome, witness) {
            let mut fresh = Worker::spawn(&self.spec.command)?;
            let replayed = fresh.call(method, witness, self.timeout)?;
            if canonical_json(&replayed) != canonical_json(value) {
                *guard = None;
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInternal,
                    Some("reducer.nondeterministic"),
                    "reducer result differs between a long-lived and a fresh worker",
                ));
            }
        }
        if outcome.is_err() {
            *guard = None;
        }
        outcome
    }

    fn discard_worker(&self) {
        *self.worker.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn spec(&self) -> &WorkerSpec {
        &self.spec
    }

    fn invoke(
        &self,
        op: &'static str,
        state: Option<&Value>,
        observation: Option<&Value>,
        effect: Option<&EffectStatus>,
        command: Option<&Value>,
        event: Option<&DomainEvent>,
    ) -> Result<WorkerResponse, Diagnostic> {
        let reducer_identity = self.spec.reducer_identity();
        let request = WorkerRequest {
            protocol: ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor: PROTOCOL_MINOR,
            },
            op,
            entrypoint: &self.spec.entrypoint,
            identity: &reducer_identity,
            state,
            observation,
            effect,
            command,
            event,
        };
        let params = serde_json::to_value(&request)
            .map_err(|error| protocol_error(format!("invalid reducer request: {error}")))?;
        let result = self.call(op, params)?;
        let mut response: WorkerResponse = serde_json::from_value(result)
            .map_err(|error| protocol_error(format!("invalid reducer response: {error}")))?;
        if response
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.code == "IK_INTERNAL")
        {
            // An unexpected reducer exception: never keep a possibly-corrupted worker.
            self.discard_worker();
        }
        if response
            .result
            .as_mut()
            .is_some_and(|result| !normalize_shared_numbers(result))
        {
            return Err(protocol_error(
                "reducer result contains a JSON integer outside the shared range [-2^63, 2^64-1]",
            ));
        }
        if response.protocol.major != PROTOCOL_MAJOR || response.protocol.minor != PROTOCOL_MINOR {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProtocolVersionUnknown,
                Some("reducer.protocol"),
                "worker protocol version mismatch",
            ));
        }
        if response.entrypoint != self.spec.entrypoint
            || response.reducer_identity != reducer_identity
        {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                Some("reducer.identity"),
                "worker response identity does not match binding",
            ));
        }
        match (
            response.ok,
            response.result.as_ref(),
            response.diagnostic.as_ref(),
        ) {
            (true, Some(_), None) => Ok(response),
            (false, None, Some(diagnostic)) => Err(translate_diagnostic(diagnostic)),
            _ => Err(protocol_error(
                "worker response must contain exactly one result or diagnostic",
            )),
        }
    }
}

impl Machine for ExternalMachine {
    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Value {
        self.initial_state.clone()
    }

    fn validate_initial(&self) -> Result<(), Diagnostic> {
        enforce_kernel_laws(&self.laws, &self.initial_state)
    }

    fn requires_capability_preflight(&self) -> bool {
        true
    }

    fn decide(
        &self,
        state: &Value,
        observation: Option<&Value>,
        effect: &EffectStatus,
        command: &Value,
    ) -> Result<Decision, Diagnostic> {
        let response = self.invoke(
            "decide",
            Some(state),
            observation,
            Some(effect),
            Some(command),
            None,
        )?;
        let result: DecisionResult = decode_result(response, "decide")?;
        match (result.decision.as_str(), result.event) {
            ("noop", None) => Ok(Decision::Noop { note: result.note }),
            ("emit", Some(event)) => Ok(Decision::Emit {
                event,
                note: result.note,
            }),
            ("noop", Some(_)) => Err(protocol_error("noop decision must not contain an event")),
            ("emit", None) => Err(protocol_error("emit decision must contain an event")),
            _ => Err(protocol_error("unknown reducer decision")),
        }
    }

    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        let response = self.invoke("apply", Some(state), None, None, None, Some(event))?;
        let result: ApplyResult = decode_result(response, "apply")?;
        enforce_kernel_laws(&self.laws, &result.state)?;
        Ok(ApplyOutcome {
            state: result.state,
            terminal: result.terminal,
            effect: result.effect,
            outcome: result.outcome,
        })
    }
}

fn decode_result<T: for<'de> Deserialize<'de>>(
    response: WorkerResponse,
    op: &str,
) -> Result<T, Diagnostic> {
    let result = response
        .result
        .ok_or_else(|| protocol_error("successful worker response is missing its result"))?;
    serde_json::from_value(result)
        .map_err(|error| protocol_error(format!("invalid {op} result: {error}")))
}

fn protocol_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IkProtocolInvalid,
        Some("reducer.protocol"),
        message,
    )
}

fn verify_code_source(spec: &WorkerSpec) -> Result<(), Diagnostic> {
    let Some(path) = spec.code_source.as_deref() else {
        return Ok(());
    };
    let bytes = std::fs::read(path).map_err(|error| {
        Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("reducer.code_source"),
            format!("code_source `{path}` could not be read: {error}"),
        )
    })?;
    let digest = sha256_hex(&bytes);
    if digest != spec.code_digest {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineMismatch,
            Some("reducer.code_digest"),
            "declared code_digest does not match hashed code_source",
        ));
    }
    Ok(())
}

fn compile_kernel_laws(specs: &[KernelLawSpec]) -> Result<Vec<CompiledKernelLaw>, Diagnostic> {
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        if spec.name.is_empty() || spec.assertion.is_empty() {
            return Err(protocol_error(
                "kernel law name and assert must be non-empty",
            ));
        }
        let when = match spec.when.as_deref() {
            None | Some("") => None,
            Some(text) => Some(crate::declarative::expr::parse(text).map_err(|error| {
                protocol_error(format!("kernel law `{}` when: {error}", spec.name))
            })?),
        };
        let assertion = crate::declarative::expr::parse(&spec.assertion).map_err(|error| {
            protocol_error(format!("kernel law `{}` assert: {error}", spec.name))
        })?;
        out.push(CompiledKernelLaw {
            name: spec.name.clone(),
            when_src: spec.when.clone(),
            when,
            assertion_src: spec.assertion.clone(),
            assertion,
        });
    }
    Ok(out)
}

fn enforce_kernel_laws(laws: &[CompiledKernelLaw], state: &Value) -> Result<(), Diagnostic> {
    if laws.is_empty() {
        return Ok(());
    }
    let facts = facts_from_state(state)?;
    let label = facts.get("state").and_then(Value::as_str).unwrap_or("");
    let args = BTreeMap::new();
    let ctx = crate::declarative::expr::EvalCtx {
        facts: &facts,
        args: &args,
        state: label,
    };
    let specs: Vec<crate::declarative::evaluate::InvariantSpec<'_>> = laws
        .iter()
        .map(|law| crate::declarative::evaluate::InvariantSpec {
            name: &law.name,
            when: law.when.as_ref(),
            when_src: law.when_src.as_deref(),
            when_rule: law.when_src.as_deref().unwrap_or(""),
            assert: &law.assertion,
            assert_rule: &law.assertion_src,
        })
        .collect();
    let checked = crate::declarative::evaluate::evaluate_invariants(&specs, &ctx);
    match checked.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn facts_from_state(state: &Value) -> Result<BTreeMap<String, Value>, Diagnostic> {
    let object = state.as_object().ok_or_else(|| {
        protocol_error("reducer state must be a JSON object to evaluate kernel laws")
    })?;
    Ok(object.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn translate_diagnostic(diagnostic: &WireDiagnostic) -> Diagnostic {
    let code = match diagnostic.code.as_str() {
        "IK_PROTOCOL_INVALID" => DiagnosticCode::IkProtocolInvalid,
        "IK_PROTOCOL_VERSION_UNKNOWN" => DiagnosticCode::IkProtocolVersionUnknown,
        "IK_MACHINE_UNKNOWN" => DiagnosticCode::IkMachineUnknown,
        "IK_MACHINE_MISMATCH" => DiagnosticCode::IkMachineMismatch,
        "IK_MACHINE_VERSION_UNKNOWN" => DiagnosticCode::IkMachineVersionUnknown,
        "IK_MACHINE_HASH_MISMATCH" => DiagnosticCode::IkMachineHashMismatch,
        "IK_EVENT_SCHEMA_UNKNOWN" => DiagnosticCode::IkEventSchemaUnknown,
        "IK_ILLEGAL_STATE" => DiagnosticCode::IkIllegalState,
        "IK_COMMAND_INVALID" => DiagnosticCode::IkCommandInvalid,
        "IK_INVARIANT_FAILED" => DiagnosticCode::IkInvariantFailed,
        "IK_EFFECT_MISMATCH" => DiagnosticCode::IkEffectMismatch,
        "IK_EFFECT_STALE" => DiagnosticCode::IkEffectStale,
        "IK_EFFECT_OUTCOME_CONFLICT" => DiagnosticCode::IkEffectOutcomeConflict,
        "IK_INTERNAL" => DiagnosticCode::IkInternal,
        other => DiagnosticCode::Domain(other.to_owned()),
    };
    Diagnostic::new(code, diagnostic.rule.clone(), diagnostic.message.clone())
}

/// A long-lived reducer worker speaking newline-delimited JSON-RPC 2.0 over stdio
/// (the language-server pattern). The worker is stateless by contract: every request carries
/// the full state, so a worker can be discarded and respawned at any time without semantic effect.
struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    replies: Receiver<Vec<u8>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    process_token: String,
    #[cfg(unix)]
    reaper_tx: Option<mpsc::Sender<()>>,
    #[cfg(unix)]
    reaper_thread: Option<thread::JoinHandle<()>>,
    next_id: u64,
    calls: u64,
}

impl Worker {
    fn spawn(argv: &[String]) -> Result<Worker, Diagnostic> {
        let process_token = Uuid::new_v4().simple().to_string();
        let mut command = Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .arg("--serve")
            .env(PROCESS_TOKEN_ENV, &process_token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                if setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // A killed `ik` must not orphan its worker.
                #[cfg(target_os = "linux")]
                {
                    const PR_SET_PDEATHSIG: i32 = 1;
                    const SIGKILL: u64 = 9;
                    prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|error| {
            Diagnostic::new(
                DiagnosticCode::IkInternal,
                Some("reducer.spawn"),
                format!("failed to launch reducer: {error}"),
            )
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let (Some(stdin), Some(stdout), Some(stderr_pipe)) = (stdin, stdout, stderr_pipe) else {
            terminate_and_reap(&mut child, &process_token);
            return Err(Diagnostic::internal("reducer stdio pipes unavailable"));
        };
        let (sender, replies) = mpsc::channel();
        thread::spawn(move || read_reply_lines(stdout, sender));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_sink = Arc::clone(&stderr);
        thread::spawn(move || capture_stderr(stderr_pipe, stderr_sink));
        #[cfg(unix)]
        let (reaper_tx, reaper_thread) = {
            let (tx, rx) = mpsc::channel::<()>();
            let child_pid = child.id();
            let token = process_token.clone();
            let handle = thread::spawn(move || {
                while rx.recv().is_ok() {
                    // Coalesce bursts (e.g., replay/verify) into a single sweep off the write-lock path.
                    while rx.try_recv().is_ok() {}
                    let mut stray = tree_targets(Some(child_pid), &token);
                    stray.remove(&child_pid);
                    kill_pids(stray);
                }
            });
            (Some(tx), Some(handle))
        };
        Ok(Worker {
            child,
            stdin: Some(stdin),
            replies,
            stderr,
            process_token,
            #[cfg(unix)]
            reaper_tx,
            #[cfg(unix)]
            reaper_thread,
            next_id: 0,
            calls: 0,
        })
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Diagnostic> {
        self.next_id += 1;
        self.calls += 1;
        let id = self.next_id;
        let mut line = canonical_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .into_bytes();
        line.push(b'\n');
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Diagnostic::internal("reducer stdin is closed"))?;
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                Diagnostic::new(
                    DiagnosticCode::IkInternal,
                    Some("reducer.stdin"),
                    format!("failed to send reducer request: {error}"),
                )
            })?;
        let raw = match self.replies.recv_timeout(timeout) {
            Ok(raw) => raw,
            Err(RecvTimeoutError::Timeout) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInternal,
                    Some("reducer.timeout"),
                    "reducer worker exceeded its timeout",
                ));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(self.exit_diagnostic()),
        };
        let mut reply: Value = serde_json::from_slice(&raw)
            .map_err(|error| protocol_error(format!("invalid reducer reply: {error}")))?;
        if reply.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || reply.get("id").and_then(Value::as_u64) != Some(id)
        {
            return Err(protocol_error(
                "reducer reply is not the matching JSON-RPC 2.0 response",
            ));
        }
        if let Some(error) = reply.get("error") {
            return Err(protocol_error(format!("reducer JSON-RPC error: {error}")));
        }
        // Reap any stray descendants off the SQLite write-lock hot path via the coalescing
        // background reaper thread so calls return immediately without waiting on `ps eww`.
        #[cfg(unix)]
        if let Some(tx) = &self.reaper_tx {
            let _ = tx.send(());
        }
        reply
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| protocol_error("reducer reply has neither result nor error"))
    }

    fn exit_diagnostic(&mut self) -> Diagnostic {
        let status = self.child.try_wait().ok().flatten();
        let detail = {
            let captured = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
            String::from_utf8_lossy(&captured).trim().to_string()
        };
        let head = match status {
            Some(status) => format!("reducer exited with {status}"),
            None => "reducer closed its output".to_string(),
        };
        Diagnostic::new(
            DiagnosticCode::IkInternal,
            Some("reducer.exit"),
            if detail.is_empty() {
                head
            } else {
                format!("{head}: {detail}")
            },
        )
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            drop(self.reaper_tx.take());
            if let Some(handle) = self.reaper_thread.take() {
                let _ = handle.join();
            }
        }
        // EOF on stdin ends the worker loop; escalate only if it does not exit promptly.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        // Cold path only: descendants must not outlive the worker.
        kill_process_tree(&mut self.child, &self.process_token);
        let _ = self.child.wait();
    }
}

fn read_reply_lines(stdout: impl Read, sender: mpsc::Sender<Vec<u8>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        match (&mut reader)
            .take(MAX_OUTPUT as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if line.len() > MAX_OUTPUT || line.last() != Some(&b'\n') {
                    break; // oversize or truncated reply: close the channel, the caller fails closed
                }
                line.pop();
                if sender.send(line).is_err() {
                    break;
                }
            }
        }
    }
}

fn capture_stderr(mut pipe: impl Read, sink: Arc<Mutex<Vec<u8>>>) {
    let mut buffer = [0_u8; 8192];
    while let Ok(count) = pipe.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let mut captured = sink.lock().unwrap_or_else(|e| e.into_inner());
        let room = MAX_OUTPUT.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..count.min(room)]);
    }
}

fn terminate_and_reap(child: &mut Child, process_token: &str) {
    terminate_process_group(child);
    let deadline = Instant::now() + TERMINATION_GRACE;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    kill_process_tree(child, process_token);
    let _ = child.wait();
}

#[cfg(unix)]
unsafe extern "C" {
    fn setpgid(pid: i32, pgid: i32) -> i32;
    fn killpg(pgid: i32, signal: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    #[cfg(target_os = "linux")]
    fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
}

fn terminate_process_group(child: &mut Child) {
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = killpg(pid, 15);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

/// Every process this worker started or that inherited its process token (including descendants
/// that detached into their own session), excluding this `ik` process.
#[cfg(unix)]
fn tree_targets(root: Option<u32>, process_token: &str) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut targets: HashSet<u32> = HashSet::new();
    let marker = format!("{PROCESS_TOKEN_ENV}={process_token}");
    // procps (Linux) rejects the dashed `-ax` in BSD form; BSD ps (macOS) rejects bare `ax`.
    let scope = if cfg!(target_os = "linux") { "ax" } else { "-ax" };
    if let Ok(output) = Command::new("ps")
        .args(["eww", scope, "-o", "pid=,ppid=,command="])
        .env_remove(PROCESS_TOKEN_ENV)
        .output()
    {
        for row in String::from_utf8_lossy(&output.stdout).lines() {
            let mut fields = row.split_whitespace();
            let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let Some(parent) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            children.entry(parent).or_default().push(pid);
            if row.contains(&marker) {
                targets.insert(pid);
            }
        }
    }
    let mut frontier: Vec<u32> = root.into_iter().collect();
    while let Some(parent) = frontier.pop() {
        if let Some(direct) = children.get(&parent) {
            targets.extend(direct.iter().copied());
            frontier.extend(direct.iter().copied());
        }
    }
    targets.remove(&std::process::id());
    targets
}

#[cfg(unix)]
fn kill_pids(targets: impl IntoIterator<Item = u32>) {
    for pid in targets {
        if let Ok(pid) = i32::try_from(pid) {
            unsafe {
                let _ = kill(pid, 9);
            }
        }
    }
}

fn kill_process_tree(child: &mut Child, process_token: &str) {
    #[cfg(unix)]
    {
        let root = matches!(child.try_wait(), Ok(None)).then(|| child.id());
        let mut targets = tree_targets(root, process_token);
        // The group operation owns root termination, never a stale PID snapshot.
        targets.remove(&child.id());
        kill_process_group(child);
        kill_pids(targets);
    }
    #[cfg(not(unix))]
    kill_process_group(child);
}

fn kill_process_group(child: &mut Child) {
    // Reaping releases the PID; it can no longer authorize a process-group signal.
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = killpg(pid, 9);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}
