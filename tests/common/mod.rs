//! Shared helpers for integration tests: machine loading and canonical envelope
//! building. Not a test file itself.
#![allow(dead_code)]

use ik::declarative::CompiledMachine;
use ik::identity::MachineIdentity;
use ik::machine::Machine;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn machine_def(machine_id: &str) -> PathBuf {
    repo_root()
        .join("machines")
        .join(format!("{machine_id}.ik.toml"))
}

pub fn load_machine(machine_id: &str) -> CompiledMachine {
    ik::declarative::load_def(&machine_def(machine_id))
        .unwrap_or_else(|errors| panic!("machine {machine_id} failed static check: {errors:?}"))
}

pub fn identity_of(machine_id: &str) -> MachineIdentity {
    load_machine(machine_id).identity().clone()
}

/// Run the `ik` binary with the given store, machine def, and request envelope.
/// Returns (stdout, exit_code).
pub fn run_cli(store: &Path, machine: Option<&Path>, request: &Value) -> (String, i32) {
    let bin = env!("CARGO_BIN_EXE_ik");
    let mut command = std::process::Command::new(bin);
    command.arg("--store").arg(store);
    if let Some(m) = machine {
        command.arg("--machine").arg(m);
    }
    command.arg(request["op"].as_str().expect("request has op"));
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("spawn ik");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(serde_json::to_string(request).unwrap().as_bytes())
            .expect("write request");
    }
    let output = child.wait_with_output().expect("wait ik");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (stdout, output.status.code().unwrap_or(-1))
}

pub fn run_cli_json(store: &Path, machine: Option<&Path>, request: &Value) -> Value {
    let (stdout, _code) = run_cli(store, machine, request);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("ik did not emit a JSON envelope: {e}; raw: {stdout}"))
}

/// Assert that an expected pattern matches a value: for objects, every expected key
/// must exist in the value and match recursively; otherwise exact equality.
pub fn assert_pattern(actual: &Value, expected: &Value, where_: &str) {
    match expected {
        Value::Object(expected_map) => {
            let actual_map = actual
                .as_object()
                .unwrap_or_else(|| panic!("{where_}: expected object, got {actual}"));
            for (key, expected_value) in expected_map {
                let actual_value = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("{where_}: missing key `{key}` in {actual}"));
                assert_pattern(actual_value, expected_value, &format!("{where_}.{key}"));
            }
        }
        Value::Array(expected_items) => {
            let actual_items = actual
                .as_array()
                .unwrap_or_else(|| panic!("{where_}: expected array, got {actual}"));
            assert_eq!(
                actual_items.len(),
                expected_items.len(),
                "{where_}: array length mismatch in {actual}"
            );
            for (i, (a, e)) in actual_items.iter().zip(expected_items.iter()).enumerate() {
                assert_pattern(a, e, &format!("{where_}[{i}]"));
            }
        }
        other => assert_eq!(actual, other, "{where_}"),
    }
}

/// Resolve `$path` references against a context of prior responses.
pub fn resolve_refs(value: &Value, ctx: &serde_json::Map<String, Value>) -> Value {
    match value {
        Value::String(s) if s.starts_with('$') => {
            let path = &s[1..];
            let mut parts = path.split('.');
            let root = parts.next().unwrap_or("");
            let root_val = ctx.get(root);
            match root_val {
                Some(v) => {
                    let mut current = v;
                    for part in parts {
                        let next = match current {
                            Value::Object(map) => map.get(part),
                            Value::Array(arr) => {
                                part.parse::<usize>().ok().and_then(|i| arr.get(i))
                            }
                            _ => None,
                        };
                        match next {
                            Some(n) => current = n,
                            None => return value.clone(),
                        }
                    }
                    current.clone()
                }
                None => value.clone(),
            }
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| resolve_refs(v, ctx)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_refs(v, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A real `ik --session` child. One canonical JSON request line in, one response line out; every
/// exchange is a blocking read on the child's stdout, so tests need no sleeps or polling.
pub struct Session {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    /// Dropping this cancels the hang watchdog.
    _watchdog: std::sync::mpsc::Sender<()>,
}

/// A regression (or a mutant) that makes `ik --session` stop answering would otherwise block the
/// whole suite forever on a read. The watchdog SIGKILLs the child after this deadline so the read
/// hits EOF and the test FAILS with a message instead of hanging.
const SESSION_HANG_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

impl Session {
    pub fn open(store: &Path, machine: Option<&Path>) -> Session {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ik"));
        command.arg("--store").arg(store);
        if let Some(machine) = machine {
            command.arg("--machine").arg(machine);
        }
        command
            .arg("--session")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().expect("spawn ik --session");
        let stdin = child.stdin.take();
        let (cancel, cancelled) = std::sync::mpsc::channel::<()>();
        let pid = child.id().to_string();
        std::thread::spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                cancelled.recv_timeout(SESSION_HANG_DEADLINE)
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid])
                    .status();
            }
        });
        let stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
        Session {
            child,
            stdin,
            stdout,
            _watchdog: cancel,
        }
    }

    /// Send one raw line (a trailing newline is added) and return the raw response line.
    pub fn exchange_line(&mut self, raw: &str) -> String {
        use std::io::{BufRead, Write};
        let stdin = self.stdin.as_mut().expect("session stdin is open");
        stdin.write_all(raw.as_bytes()).expect("write request");
        stdin.write_all(b"\n").expect("write newline");
        stdin.flush().expect("flush request");
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read response");
        assert!(read > 0, "ik --session closed its output unexpectedly");
        line.trim_end_matches('\n').to_string()
    }

    pub fn request(&mut self, request: &Value) -> Value {
        let raw = self.exchange_line(&serde_json::to_string(request).unwrap());
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("session did not emit a JSON envelope: {e}; raw: {raw}"))
    }

    /// Close stdin (EOF) and return the exit code.
    pub fn close(mut self) -> i32 {
        drop(self.stdin.take());
        self.child
            .wait()
            .expect("wait ik --session")
            .code()
            .unwrap_or(-1)
    }

    /// SIGKILL the session without letting it flush anything (crash model).
    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
