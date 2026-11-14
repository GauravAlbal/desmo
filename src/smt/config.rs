//! Optional pinned solver adapter. Absent configuration leaves `ik check` unchanged.

use super::solve::{probe_named, SolverBin};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

const ADAPTER_ENV: &str = "IK_SMT_ADAPTER";
const PLACEHOLDER: &str = "0000000000000000000000000000000000000000000000000000000000000000";

thread_local! {
    static THREAD_ADAPTER: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

/// Per-thread adapter override so tests do not race on the process environment.
pub struct ThreadAdapter;

impl ThreadAdapter {
    pub fn auto() -> Self {
        set_thread_adapter(Some("auto"));
        Self
    }
}

impl Drop for ThreadAdapter {
    fn drop(&mut self) {
        clear_thread_adapter();
    }
}

pub fn set_thread_adapter(value: Option<&str>) {
    let stored = value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    THREAD_ADAPTER.with(|slot| {
        *slot.borrow_mut() = Some(stored);
    });
}

pub fn clear_thread_adapter() {
    THREAD_ADAPTER.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

fn adapter_value() -> Option<String> {
    THREAD_ADAPTER.with(|slot| match slot.borrow().as_ref() {
        Some(overridden) => overridden.clone(),
        None => std::env::var(ADAPTER_ENV)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
    })
}

/// True only when the operator explicitly pointed at an adapter directory or `auto`.
pub fn adapter_configured() -> bool {
    adapter_value().is_some()
}

/// Load Z3 and cvc5 from the pinned toolchain, or from `auto` discovery.
pub fn configured_solvers() -> (Option<SolverBin>, Option<SolverBin>) {
    let Some(value) = adapter_value() else {
        return (None, None);
    };
    if value == "auto" {
        return super::solve::discover_solvers();
    }
    load_toolchain(Path::new(&value)).0
}

/// Why an explicit adapter loads nothing or less than configured. Empty for `auto`,
/// no adapter, or a fully pinned toolchain. Surfaced so a misconfigured adapter is
/// never silently equivalent to no adapter.
pub fn adapter_problems() -> Vec<String> {
    match adapter_value() {
        Some(value) if value != "auto" => load_toolchain(Path::new(&value)).1,
        _ => Vec::new(),
    }
}

type Loaded = (Option<SolverBin>, Option<SolverBin>);

fn load_toolchain(path: &Path) -> (Loaded, Vec<String>) {
    let file = if path.is_dir() {
        path.join("toolchain.json")
    } else {
        path.to_path_buf()
    };
    let value = match read_toolchain(&file) {
        Ok(value) => value,
        Err(err) => return ((None, None), vec![err]),
    };
    let mut problems = Vec::new();
    let z3 = solver_from_toolchain(&value, "z3", &mut problems);
    let cvc5 = solver_from_toolchain(&value, "cvc5", &mut problems);
    if z3.is_none() && cvc5.is_none() && problems.is_empty() {
        problems.push("toolchain names no solver".to_string());
    }
    ((z3, cvc5), problems)
}

fn read_toolchain(file: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|err| format!("read {}: {err}", file.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("toolchain json: {err}"))
}

/// An explicit toolchain entry is used only with a real pinned digest that matches the
/// executable bytes. Placeholder paths and placeholder digests load nothing and say so.
fn solver_from_toolchain(
    value: &serde_json::Value,
    name: &str,
    problems: &mut Vec<String>,
) -> Option<SolverBin> {
    let path = match value.get(name).and_then(|row| row.as_str()) {
        Some(text) if !text.is_empty() && !text.starts_with("/absolute/path/") => {
            PathBuf::from(text)
        }
        _ => {
            problems.push(format!("{name}: no executable path configured"));
            return None;
        }
    };
    let digest_key = format!("{name}_sha256");
    let expected = value
        .get(digest_key.as_str())
        .and_then(|row| row.as_str())
        .unwrap_or(PLACEHOLDER);
    if expected == PLACEHOLDER {
        problems.push(format!("{name}: digest not pinned"));
        return None;
    }
    match sha256_file(&path) {
        Ok(digest) if digest == expected => {}
        Ok(_) => {
            problems.push(format!("{name}: digest mismatch for {}", path.display()));
            return None;
        }
        Err(err) => {
            problems.push(err);
            return None;
        }
    }
    let bin = probe_named(name, &path);
    if bin.is_none() {
        problems.push(format!("{name}: {} did not answer the version probe", path.display()));
    }
    bin
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    Ok(crate::canonical::sha256_hex(&bytes))
}
