//! Explicit promotion of a scenario bundle into a regression case.
//!
//! Promotion copies the bundle. It never rewrites expectations to make a
//! failing case pass, and it refuses a bundle whose SHA256SUMS do not verify.

use crate::canonical::{canonical_json, sha256_hex};
use crate::scenario::{reproduce_bundle, Reproduction};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
usage: ik regression promote <bundle-dir> --name <name> --reason <reason> [--into <dir>]
       ik regression run <dir>

promote copies scenario.json, machine.ik.toml, result.json, SHA256SUMS, and
any other file named by SHA256SUMS into <into>/<name>/ (default regressions/)
and writes REGRESSION.json. It refuses to overwrite and refuses a bundle whose
SHA256SUMS do not verify. run re-executes every promoted bundle and prints
reproduced or not_reproduced. Exit 1 if any case is not reproduced, 2 on usage.
";

const REQUIRED: &[&str] = &["scenario.json", "machine.ik.toml", "result.json"];

struct SumLine {
    digest: String,
    name: String,
}

struct PromoteArgs {
    bundle: PathBuf,
    name: String,
    reason: String,
    into: PathBuf,
}

enum Invocation {
    Help,
    Promote(PromoteArgs),
    Run(PathBuf),
}

/// CLI entry for `ik regression`.
pub fn run(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(Invocation::Help) => {
            println!("{USAGE}");
            0
        }
        Ok(Invocation::Promote(args)) => match promote(&args) {
            Ok(dest) => {
                println!("promoted {}", dest.display());
                0
            }
            Err(message) => usage_error(&message),
        },
        Ok(Invocation::Run(dir)) => match run_cases(&dir) {
            Ok(code) => code,
            Err(message) => usage_error(&message),
        },
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik regression: {message}");
    eprintln!("{USAGE}");
    2
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(Invocation::Help);
    }
    if args.is_empty() {
        return Err("regression command is required".into());
    }
    match args[0].as_str() {
        "promote" => parse_promote(&args[1..]).map(Invocation::Promote),
        "run" => parse_run(&args[1..]).map(Invocation::Run),
        other => Err(format!("unknown regression command `{other}`")),
    }
}

fn parse_promote(args: &[String]) -> Result<PromoteArgs, String> {
    let mut name = None;
    let mut reason = None;
    let mut into = None;
    let mut bundle = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--name" => name = Some(take_value(args, &mut index, "--name")?),
            "--reason" => reason = Some(take_value(args, &mut index, "--reason")?),
            "--into" => into = Some(PathBuf::from(take_value(args, &mut index, "--into")?)),
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            positional if bundle.is_none() => bundle = Some(PathBuf::from(positional)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        index += 1;
    }
    let name = name.ok_or_else(|| "--name is required".to_string())?;
    let reason = reason.ok_or_else(|| "--reason is required".to_string())?;
    if reason.trim().is_empty() {
        return Err("--reason must not be empty".into());
    }
    if !valid_name(&name) {
        return Err(format!("invalid regression name `{name}`"));
    }
    Ok(PromoteArgs {
        bundle: bundle.ok_or_else(|| "promote requires <bundle-dir>".to_string())?,
        name,
        reason,
        into: into.unwrap_or_else(|| PathBuf::from("regressions")),
    })
}

fn parse_run(args: &[String]) -> Result<PathBuf, String> {
    if args.len() != 1 || args[0].starts_with('-') {
        return Err("run requires <dir>".into());
    }
    Ok(PathBuf::from(&args[0]))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
}

fn promote(args: &PromoteArgs) -> Result<PathBuf, String> {
    if !args.bundle.is_dir() {
        return Err(format!("not a bundle directory: {}", args.bundle.display()));
    }
    let sums = verify_sums(&args.bundle)?;
    let spec_hash = spec_hash_of(&args.bundle)?;
    let dest = args.into.join(&args.name);
    if dest.exists() {
        return Err(format!("refusing to overwrite {}", dest.display()));
    }
    fs::create_dir_all(&args.into)
        .map_err(|err| format!("create {}: {err}", args.into.display()))?;
    if dest.exists() {
        return Err(format!("refusing to overwrite {}", dest.display()));
    }
    let staging = staging_dir(&args.into, &args.name);
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|err| format!("remove {}: {err}", staging.display()))?;
    }
    fs::create_dir(&staging).map_err(|err| format!("create {}: {err}", staging.display()))?;
    if let Err(err) = write_promoted(&args.bundle, &staging, &sums, args, &spec_hash) {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }
    if dest.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("refusing to overwrite {}", dest.display()));
    }
    fs::rename(&staging, &dest).map_err(|err| {
        let _ = fs::remove_dir_all(&staging);
        format!("rename {} to {}: {err}", staging.display(), dest.display())
    })?;
    Ok(dest)
}

fn write_promoted(
    bundle: &Path,
    staging: &Path,
    sums: &[SumLine],
    args: &PromoteArgs,
    spec_hash: &str,
) -> Result<(), String> {
    for line in sums {
        copy_file(&bundle.join(&line.name), &staging.join(&line.name))?;
    }
    copy_file(&bundle.join("SHA256SUMS"), &staging.join("SHA256SUMS"))?;
    let digest = sha256_hex(&read_bytes(&bundle.join("SHA256SUMS"))?);
    let document = json!({
        "schema": "ik.regression.v1",
        "name": args.name,
        "reason": args.reason,
        "source_bundle_digest": digest,
        "spec_hash": spec_hash,
        "runtime_binary_sha256": current_binary_sha()?,
    });
    fs::write(staging.join("REGRESSION.json"), canonical_json(&document))
        .map_err(|err| format!("write REGRESSION.json: {err}"))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map_err(|err| format!("copy {} to {}: {err}", from.display(), to.display()))?;
    Ok(())
}

fn staging_dir(into: &Path, name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    into.join(format!(".{name}.{}.{nanos}.partial", std::process::id()))
}

fn verify_sums(dir: &Path) -> Result<Vec<SumLine>, String> {
    let text = fs::read_to_string(dir.join("SHA256SUMS"))
        .map_err(|err| format!("read SHA256SUMS: {err}"))?;
    let lines = parse_sums(&text)?;
    if lines.is_empty() {
        return Err("SHA256SUMS is empty".into());
    }
    for required in REQUIRED {
        if !lines.iter().any(|line| line.name == *required) {
            return Err(format!("SHA256SUMS missing {required}"));
        }
    }
    for line in &lines {
        let actual = sha256_hex(&read_bytes(&dir.join(&line.name))?);
        if actual != line.digest {
            return Err(format!(
                "SHA256SUMS verification failed: {} digest mismatch",
                line.name
            ));
        }
    }
    Ok(lines)
}

fn parse_sums(text: &str) -> Result<Vec<SumLine>, String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        lines.push(parse_sum_line(raw)?);
    }
    let mut names: Vec<&str> = lines.iter().map(|line| line.name.as_str()).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    if names.len() != before {
        return Err("SHA256SUMS has a duplicate name".into());
    }
    Ok(lines)
}

fn parse_sum_line(line: &str) -> Result<SumLine, String> {
    let mut parts = line.split_whitespace();
    let digest = parts
        .next()
        .ok_or_else(|| "SHA256SUMS line is missing a digest".to_string())?;
    let name = parts
        .next()
        .ok_or_else(|| "SHA256SUMS line is missing a name".to_string())?;
    if parts.next().is_some() {
        return Err(format!("SHA256SUMS line has extra fields: {line}"));
    }
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("SHA256SUMS digest is not sha256: {name}"));
    }
    if !safe_sum_name(name) {
        return Err(format!("SHA256SUMS name is not a bundle file: {name}"));
    }
    Ok(SumLine {
        digest: digest.to_ascii_lowercase(),
        name: name.to_string(),
    })
}

fn safe_sum_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
}

fn spec_hash_of(dir: &Path) -> Result<String, String> {
    let value: Value = serde_json::from_slice(&read_bytes(&dir.join("result.json"))?)
        .map_err(|err| format!("result.json: {err}"))?;
    if field_str(&value, "schema") != Some("ik.scenario.result.v1") {
        return Err("result.json is not ik.scenario.result.v1".into());
    }
    field_str(&value, "spec_hash")
        .map(str::to_string)
        .ok_or_else(|| "result.json has no spec_hash".to_string())
}

fn current_binary_sha() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|err| format!("current executable: {err}"))?;
    Ok(sha256_hex(&read_bytes(&path)?))
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))
}

fn field_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn run_cases(dir: &Path) -> Result<i32, String> {
    let cases = case_dirs(dir)?;
    let mut failed = false;
    for case in cases {
        if !run_case(&case)? {
            failed = true;
        }
    }
    Ok(if failed { 1 } else { 0 })
}

fn case_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    if dir.join("REGRESSION.json").is_file() {
        return Ok(vec![dir.to_path_buf()]);
    }
    let mut found = Vec::new();
    let entries = fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() && path.join("REGRESSION.json").is_file() {
            found.push(path);
        }
    }
    found.sort();
    if found.is_empty() {
        return Err(format!("no promoted bundles in {}", dir.display()));
    }
    Ok(found)
}

fn run_case(case: &Path) -> Result<bool, String> {
    let name = case
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or("case");
    let scratch = scratch_dir(name)?;
    let outcome = reproduce_bundle(case, &scratch);
    let _ = fs::remove_dir_all(&scratch);
    match outcome {
        Ok(report) => {
            print_case(name, &report);
            Ok(report.reproduced)
        }
        Err(err) => {
            println!("{name}: not_reproduced");
            println!("{err}");
            Ok(false)
        }
    }
}

fn print_case(name: &str, report: &Reproduction) {
    let status = if report.reproduced {
        "reproduced"
    } else {
        "not_reproduced"
    };
    println!("{name}: {status}");
    if report.reproduced {
        return;
    }
    for line in report.lines.iter().skip(1) {
        println!("  {line}");
    }
}

fn scratch_dir(name: &str) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "ik-regression-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).map_err(|err| format!("create {}: {err}", path.display()))?;
    Ok(path)
}
