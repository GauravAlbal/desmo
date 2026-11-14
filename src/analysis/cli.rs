//! `ik analysis` command surface.
//!
//! `describe` reads a manifest and does not execute it. `run` spawns an
//! adapter. `conformance` spawns that same path once per fixture.
//! `validate-result` rechecks a stored result against its request and the
//! current input bytes.

use super::manifest::parse_manifest;
use super::paths::is_symlink;
use super::request::{self, snapshot_inputs};
use super::result::validate_result_bytes;
use super::runner::{controls_for, run_adapter, HostReport, RunSpec};
use crate::canonical::canonical_json;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Serialize)]
pub struct DescribeReport {
    protocol: &'static str,
    executed: bool,
    rejection_reason: Option<String>,
    adapter: Option<Value>,
    engine: Option<Value>,
    invocation: Option<Value>,
    inputs: Option<Vec<String>>,
    properties: Option<Vec<String>>,
    evidence: Option<Vec<String>>,
    requires: Option<Value>,
    result_schema: Option<String>,
}

pub fn run_cli(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("describe") => describe_cli(&args[1..]),
        Some("run") => run_command(&args[1..]),
        Some("validate-result") => validate_cli(&args[1..]),
        Some("conformance") => conformance_cli(&args[1..]),
        _ => usage(),
    }
}

fn usage() -> i32 {
    eprintln!("usage: ik analysis describe <manifest.json>");
    eprintln!("       ik analysis run <manifest.json> --request <request.json> --out <dir> [--timeout-seconds N]");
    eprintln!("       ik analysis validate-result <request.json> <result.json>");
    eprintln!("       ik analysis conformance <manifest.json> [--fixtures <dir>]");
    2
}

fn describe_cli(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        return usage();
    };
    if args.len() != 1 {
        return usage();
    }
    let report = describe_manifest(Path::new(path));
    println!("{}", canonical_json(&report));
    if report.rejection_reason.is_none() {
        0
    } else {
        2
    }
}

pub fn describe_manifest(path: &Path) -> DescribeReport {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return describe_rejection("malformed_manifest"),
    };
    match parse_manifest(&text) {
        Ok(manifest) => DescribeReport {
            protocol: "ik.analysis.describe.v1",
            executed: false,
            rejection_reason: None,
            adapter: Some(json_id(&manifest.adapter.id, &manifest.adapter.version)),
            engine: Some(serde_json::json!({
                "id": manifest.engine.id,
                "version_constraint": manifest.engine.version_constraint,
            })),
            invocation: Some(serde_json::json!({
                "argv": manifest.invocation.argv,
                "executable_sha256": manifest.invocation.executable_sha256,
            })),
            inputs: Some(manifest.inputs),
            properties: Some(manifest.properties),
            evidence: Some(manifest.evidence),
            requires: Some(serde_json::json!({
                "production_store_access": manifest.requires.production_store_access,
                "network": manifest.requires.network,
            })),
            result_schema: Some(manifest.result_schema),
        },
        Err(reason) => describe_rejection(&reason),
    }
}

fn describe_rejection(reason: &str) -> DescribeReport {
    DescribeReport {
        protocol: "ik.analysis.describe.v1",
        executed: false,
        rejection_reason: Some(reason.to_string()),
        adapter: None,
        engine: None,
        invocation: None,
        inputs: None,
        properties: None,
        evidence: None,
        requires: None,
        result_schema: None,
    }
}

fn json_id(id: &str, version: &str) -> Value {
    serde_json::json!({"id": id, "version": version})
}

fn run_command(args: &[String]) -> i32 {
    let parsed = match parse_run_args(args) {
        Ok(parsed) => parsed,
        Err(()) => return usage(),
    };
    let report = run_adapter(&RunSpec {
        manifest_path: parsed.manifest,
        request_path: parsed.request,
        out_dir: parsed.out,
        timeout_override: parsed.timeout,
        cancel: None,
    });
    println!("{}", canonical_json(&report));
    exit_code(&report)
}

struct RunArgs {
    manifest: PathBuf,
    request: PathBuf,
    out: PathBuf,
    timeout: Option<Duration>,
}

fn parse_run_args(args: &[String]) -> Result<RunArgs, ()> {
    let mut parsed = RunParts::default();
    let mut index = 0;
    while index < args.len() {
        index = take_run_arg(args, index, &mut parsed)?;
    }
    finish_run_args(parsed)
}

#[derive(Default)]
struct RunParts {
    manifest: Option<PathBuf>,
    request: Option<PathBuf>,
    out: Option<PathBuf>,
    timeout: Option<Duration>,
}

fn finish_run_args(parsed: RunParts) -> Result<RunArgs, ()> {
    Ok(RunArgs {
        manifest: parsed.manifest.ok_or(())?,
        request: parsed.request.ok_or(())?,
        out: parsed.out.ok_or(())?,
        timeout: parsed.timeout,
    })
}

fn take_run_arg(args: &[String], mut index: usize, parsed: &mut RunParts) -> Result<usize, ()> {
    let arg = args.get(index).map(String::as_str).ok_or(())?;
    if arg == "--request" {
        parsed.request = Some(required_value(args, &mut index)?);
        return Ok(index);
    }
    if arg == "--out" {
        parsed.out = Some(required_value(args, &mut index)?);
        return Ok(index);
    }
    if arg == "--timeout-seconds" {
        parsed.timeout = Some(parse_timeout(args, &mut index)?);
        return Ok(index);
    }
    take_manifest_arg(arg, index, parsed)
}

fn take_manifest_arg(arg: &str, index: usize, parsed: &mut RunParts) -> Result<usize, ()> {
    if arg.starts_with('-') || parsed.manifest.is_some() {
        return Err(());
    }
    parsed.manifest = Some(PathBuf::from(arg));
    Ok(index + 1)
}

fn required_value(args: &[String], index: &mut usize) -> Result<PathBuf, ()> {
    let value = args.get(*index + 1).ok_or(())?;
    *index += 2;
    Ok(PathBuf::from(value))
}

fn parse_timeout(args: &[String], index: &mut usize) -> Result<Duration, ()> {
    let value = args.get(*index + 1).ok_or(())?;
    *index += 2;
    let seconds: u64 = value.parse().map_err(|_| ())?;
    if seconds == 0 {
        return Err(());
    }
    Ok(Duration::from_secs(seconds))
}

fn validate_cli(args: &[String]) -> i32 {
    if args.len() != 2 {
        return usage();
    }
    let report = validate_stored(Path::new(&args[0]), Path::new(&args[1]));
    println!("{}", canonical_json(&report));
    if report.result_accepted {
        0
    } else {
        1
    }
}

pub fn validate_stored(request_path: &Path, result_path: &Path) -> HostReport {
    let controls = controls_for(false, 0, 0, false);
    let request = match request::load_request(request_path) {
        Ok(request) => request,
        Err(reason) => {
            return super::runner::rejected_report(&reason, None, false, &controls);
        }
    };
    if is_symlink(result_path) {
        return super::runner::rejected_report(
            "symlink_rejected",
            Some(&request),
            false,
            &controls,
        );
    }
    let bytes = match fs::read_to_string(result_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return super::runner::rejected_report(
                "malformed_json",
                Some(&request),
                false,
                &controls,
            );
        }
    };
    let adapter_out = result_path.parent().unwrap_or(Path::new("."));
    let snapshots = match snapshot_inputs(&request) {
        Ok(snapshots) => snapshots,
        Err(reason) => {
            return super::runner::rejected_report(&reason, Some(&request), false, &controls);
        }
    };
    match validate_result_bytes(&request, &bytes, adapter_out, &snapshots, &snapshots) {
        Ok(result) => super::runner::accepted_report(&request, result, None, &controls),
        Err(reason) => super::runner::rejected_report(&reason, Some(&request), false, &controls),
    }
}

fn exit_code(report: &HostReport) -> i32 {
    if report.result_accepted {
        0
    } else if report.refused_before_execution {
        2
    } else {
        1
    }
}

struct ConformanceArgs {
    manifest: PathBuf,
    fixtures: Option<PathBuf>,
}

fn conformance_cli(args: &[String]) -> i32 {
    let parsed = match parse_conformance_args(args) {
        Ok(parsed) => parsed,
        Err(()) => return usage(),
    };
    let report = super::conformance::run(&parsed.manifest, parsed.fixtures.as_deref());
    println!("{}", canonical_json(&report));
    conformance_exit(&report)
}

fn parse_conformance_args(args: &[String]) -> Result<ConformanceArgs, ()> {
    if args.is_empty() || args.len() > 3 {
        return Err(());
    }
    let manifest = PathBuf::from(&args[0]);
    if args.len() == 1 {
        return Ok(ConformanceArgs {
            manifest,
            fixtures: None,
        });
    }
    if args.get(1).map(String::as_str) != Some("--fixtures") || args.len() != 3 {
        return Err(());
    }
    Ok(ConformanceArgs {
        manifest,
        fixtures: Some(PathBuf::from(&args[2])),
    })
}

fn conformance_exit(report: &super::conformance::ConformanceReport) -> i32 {
    if report.error.is_some() {
        2
    } else if report.passed {
        0
    } else {
        1
    }
}
