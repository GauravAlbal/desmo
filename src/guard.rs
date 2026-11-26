//! CI anti-bypass guardrail (`ik guard`) for brownfield repositories.
//!
//! Enforces `.ik/guard.toml` rules so coding agents and developers cannot silently
//! re-introduce direct status/flag writes outside explicitly authorized projection modules.
//! Inline ignore comments inside application source files are intentionally unsupported;
//! exemptions live only in `allow_paths` inside `.ik/guard.toml`.

use crate::canonical::canonical_json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

const USAGE: &str =
    "usage: ik guard init [--machine <m.ik.toml>] [--table <name>] [--out <path>] [--force] | check [--config <path>] [--dir <path>] [--json]";
const SCHEMA: &str = "ik.guard.v1";
const DEFAULT_CONFIG: &str = ".ik/guard.toml";

const STARTER_GUARD_TOML: &str = r#"# Declarative CI anti-bypass guard for Desmo brownfield adoptions.
# Exemptions belong in `allow_paths` here; inline ignore comments in source files are ignored.
guard_version = 1

[[rules]]
id = "orders-status-authority"
description = "Direct order status writes must go through the Desmo projection module"
case_insensitive = true
forbidden_patterns = [
  "UPDATE orders SET *status*",
  ".status = \"paid\"",
  ".status = \"refunded\"",
  ".status = 'paid'",
  ".status = 'refunded'",
  "status: \"paid\"",
  "status: 'paid'",
  "status: \"refunded\"",
  "status: 'refunded'"
]
include_globs = ["*.ts", "*.tsx", "*.js", "*.mjs", "*.py", "*.go", "*.sql"]
allow_paths = ["src/projections/*", "tests/*"]
"#;

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardConfig {
    pub guard_version: u64,
    pub rules: Vec<GuardRule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardRule {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    pub forbidden_patterns: Vec<String>,
    #[serde(default = "default_include_globs")]
    pub include_globs: Vec<String>,
    #[serde(default)]
    pub allow_paths: Vec<String>,
    #[serde(default = "default_true")]
    pub case_insensitive: bool,
}

fn default_include_globs() -> Vec<String> {
    vec![
        "*.ts".into(),
        "*.tsx".into(),
        "*.js".into(),
        "*.mjs".into(),
        "*.py".into(),
        "*.go".into(),
        "*.sql".into(),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GuardViolation {
    pub rule_id: String,
    pub file: String,
    pub line: usize,
    pub pattern: String,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GuardReport {
    pub ok: bool,
    pub rules_checked: usize,
    pub files_scanned: usize,
    pub violations: Vec<GuardViolation>,
}

pub fn load_config(path: &Path) -> Result<GuardConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("read guard config {}: {e}", path.display()))?;
    let cfg: GuardConfig = toml::from_str(&raw)
        .map_err(|e| format!("parse guard config {}: {e}", path.display()))?;
    if cfg.guard_version != 1 {
        return Err(format!(
            "unsupported guard_version {} in {} (expected 1)",
            cfg.guard_version,
            path.display()
        ));
    }
    if cfg.rules.is_empty() {
        return Err(format!(
            "guard config {} must declare at least one [[rules]] entry",
            path.display()
        ));
    }
    for (idx, rule) in cfg.rules.iter().enumerate() {
        if rule.id.trim().is_empty() {
            return Err(format!("guard rule #{idx} must declare a non-empty `id`"));
        }
        if rule.forbidden_patterns.is_empty() {
            return Err(format!(
                "guard rule `{}` must declare at least one forbidden pattern",
                rule.id
            ));
        }
        if rule.forbidden_patterns.iter().any(|p| p.is_empty()) {
            return Err(format!(
                "guard rule `{}` contains an empty forbidden pattern",
                rule.id
            ));
        }
    }
    Ok(cfg)
}

/// Scan `root` deterministically against `cfg` (ignoring `config_path` itself).
pub fn check_dir(root: &Path, cfg: &GuardConfig, config_path: &Path) -> Result<GuardReport, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let config_canon = std::fs::canonicalize(config_path).ok();
    let mut files = Vec::new();
    collect_files(root, root, config_canon.as_deref(), &mut files)?;
    files.sort();

    let mut violations = Vec::new();
    let mut files_scanned = 0usize;

    for (rel, abs) in files {
        let base = abs
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(rel.as_str());
        let matching_rules: Vec<&GuardRule> = cfg
            .rules
            .iter()
            .filter(|rule| {
                rule.include_globs
                    .iter()
                    .any(|g| match_include_glob(g, &rel, base))
                    && !rule
                        .allow_paths
                        .iter()
                        .any(|a| match_path_glob(a, &rel))
            })
            .collect();
        if matching_rules.is_empty() {
            continue;
        }
        let Some(content) = read_utf8_if_text(&abs)? else {
            continue;
        };
        files_scanned += 1;
        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("");
        let stripped = strip_comments_preserve_lines(&content, ext);
        let raw_lines: Vec<&str> = content.lines().collect();
        let (collapsed_raw, line_map) = normalize_whitespace_with_lines(&stripped);
        let collapsed_chars: Vec<char> = collapsed_raw.chars().collect();
        let collapsed_lower: Vec<char> = collapsed_raw.to_lowercase().chars().collect();

        let mut seen_hits: HashSet<(String, usize, String)> = HashSet::new();

        for (idx, line) in stripped.lines().enumerate() {
            let line_no = idx + 1;
            let raw_snippet = raw_lines.get(idx).copied().unwrap_or(line);
            let line_chars: Vec<char> = line.chars().collect();
            let line_lower: Vec<char> = line.to_lowercase().chars().collect();
            for rule in &matching_rules {
                let txt = if rule.case_insensitive {
                    &line_lower
                } else {
                    &line_chars
                };
                for pat in &rule.forbidden_patterns {
                    let pat_chars: Vec<char> = if rule.case_insensitive {
                        pat.to_lowercase().chars().collect()
                    } else {
                        pat.chars().collect()
                    };
                    if find_wildcard_start(&pat_chars, txt).is_some()
                        && seen_hits.insert((rule.id.clone(), line_no, pat.clone()))
                    {
                        violations.push(GuardViolation {
                            rule_id: rule.id.clone(),
                            file: rel.clone(),
                            line: line_no,
                            pattern: pat.clone(),
                            snippet: trim_snippet(raw_snippet),
                        });
                    }
                }
            }
        }

        // Also match patterns across formatted multi-line SQL / ORM calls using comment-stripped,
        // whitespace-collapsed source so wrapped statements cannot evade single-line scanning.
        if !collapsed_chars.is_empty() {
            for rule in &matching_rules {
                let txt = if rule.case_insensitive {
                    &collapsed_lower
                } else {
                    &collapsed_chars
                };
                for pat in &rule.forbidden_patterns {
                    let pat_chars: Vec<char> = if rule.case_insensitive {
                        pat.to_lowercase().chars().collect()
                    } else {
                        pat.chars().collect()
                    };
                    let mut search_from = 0usize;
                    while search_from <= txt.len() {
                        let Some(rel_idx) = find_wildcard_start(&pat_chars, &txt[search_from..])
                        else {
                            break;
                        };
                        let abs_idx = search_from + rel_idx;
                        let line_no = *line_map.get(abs_idx).unwrap_or(&1);
                        let raw_snippet = raw_lines
                            .get(line_no.saturating_sub(1))
                            .copied()
                            .unwrap_or("");
                        if seen_hits.insert((rule.id.clone(), line_no, pat.clone())) {
                            violations.push(GuardViolation {
                                rule_id: rule.id.clone(),
                                file: rel.clone(),
                                line: line_no,
                                pattern: pat.clone(),
                                snippet: trim_snippet(raw_snippet),
                            });
                        }
                        search_from = abs_idx + 1;
                    }
                }
            }
        }
    }

    Ok(GuardReport {
        ok: violations.is_empty(),
        rules_checked: cfg.rules.len(),
        files_scanned,
        violations,
    })
}

fn trim_snippet(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= 140 {
        trimmed.to_string()
    } else {
        let short: String = trimmed.chars().take(137).collect();
        format!("{short}...")
    }
}

fn collect_files(
    root: &Path,
    dir: &Path,
    config_canon: Option<&Path>,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read dir entry: {e}"))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if matches!(
                name,
                ".git"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".next"
            ) {
                continue;
            }
            collect_files(root, &path, config_canon, out)?;
        } else if path.is_file() {
            if let Some(cfg_c) = config_canon {
                if let Ok(p_c) = std::fs::canonicalize(&path) {
                    if p_c == cfg_c {
                        continue;
                    }
                }
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if rel == ".ik/guard.toml" {
                continue;
            }
            out.push((rel, path));
        }
    }
    Ok(())
}

fn read_utf8_if_text(path: &Path) -> Result<Option<String>, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut probe = [0u8; 8192];
    let n = file
        .read(&mut probe)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    if probe[..n].contains(&0) {
        return Ok(None);
    }
    let mut bytes = probe[..n].to_vec();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(String::from_utf8(bytes).ok())
}

fn match_include_glob(glob: &str, rel_path: &str, basename: &str) -> bool {
    if glob.contains('/') {
        match_path_glob(glob, rel_path)
    } else {
        wildcard_match(glob, basename, false)
    }
}

fn match_path_glob(glob: &str, path: &str) -> bool {
    let norm = glob.trim_start_matches("./").replace('\\', "/");
    if norm.contains("**") {
        let collapsed = norm.replace("**/", "*");
        wildcard_match(&collapsed, path, true)
    } else {
        wildcard_match(&norm, path, false)
    }
}

/// Strip comments outside string/template literals while preserving line breaks and string contents.
fn strip_comments_preserve_lines(content: &str, ext: &str) -> String {
    let is_py = ext.eq_ignore_ascii_case("py");
    let is_sql = ext.eq_ignore_ascii_case("sql");
    let has_c_comments = matches!(
        ext.to_ascii_lowercase().as_str(),
        "ts" | "tsx" | "js" | "mjs" | "go" | "sql"
    );

    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut i = 0usize;
    let mut quote: Option<char> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < chars.len() {
        let c = chars[i];
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                out.push('\n');
            } else {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                in_block_comment = false;
                out.push_str("  ");
                i += 2;
            } else if c == '\n' {
                out.push('\n');
                i += 1;
            } else {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            out.push(c);
            if !is_sql && c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }

        if is_py && c == '#' {
            in_line_comment = true;
            out.push(' ');
            i += 1;
            continue;
        }
        if is_sql && c == '-' && chars.get(i + 1) == Some(&'-') {
            in_line_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if !is_py && !is_sql && c == '/' && chars.get(i + 1) == Some(&'/') {
            in_line_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if has_c_comments && c == '/' && chars.get(i + 1) == Some(&'*') {
            in_block_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if c == '\'' || c == '"' || (!is_py && !is_sql && c == '`') {
            quote = Some(c);
        }
        out.push(c);
        i += 1;
    }
    out
}

fn normalize_whitespace_with_lines(text: &str) -> (String, Vec<usize>) {
    let mut collapsed = String::with_capacity(text.len());
    let mut line_map = Vec::with_capacity(text.len());
    let mut line_no = 1usize;
    let mut prev_space = true;
    for ch in text.chars() {
        if ch == '\n' {
            if !prev_space {
                collapsed.push(' ');
                line_map.push(line_no);
                prev_space = true;
            }
            line_no += 1;
        } else if ch.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                line_map.push(line_no);
                prev_space = true;
            }
        } else {
            collapsed.push(ch);
            line_map.push(line_no);
            prev_space = false;
        }
    }
    (collapsed, line_map)
}

fn wildcard_match(pattern: &str, text: &str, line_mode: bool) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    if line_mode {
        return find_wildcard_start(&pat, &txt).is_some();
    }
    match_full_path_glob(&pat, &txt)
}

fn find_wildcard_start(pat: &[char], txt: &[char]) -> Option<usize> {
    if pat.is_empty() {
        return Some(0);
    }
    let first_non_star = pat[0] != '*';
    for start in 0..=txt.len() {
        if first_non_star && txt.get(start) != Some(&pat[0]) {
            continue;
        }
        if match_anchored_crossing(pat, &txt[start..]) {
            return Some(start);
        }
    }
    None
}

fn match_anchored_crossing(pat: &[char], txt: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < txt.len() {
        if pi < pat.len() && pat[pi] == txt[ti] {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
        // In substring line mode, once the entire pattern is matched, succeed early.
        if pi == pat.len() {
            return true;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

fn match_full_path_glob(pat: &[char], txt: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == txt[ti] || pat[pi] == '?') {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            if txt[star_ti] == '/' {
                return false;
            }
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// CLI entry for `ik guard`.
pub fn run(args: &[String]) -> i32 {
    if args.is_empty() || matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!("{USAGE}");
        return 0;
    }
    match args[0].as_str() {
        "init" => run_init(&args[1..]),
        "check" => run_check(&args[1..]),
        other => {
            eprintln!("ik guard: unknown subcommand `{other}` (expected `init` or `check`)");
            eprintln!("{USAGE}");
            2
        }
    }
}

fn guard_toml_for_machine(machine_path: &Path, table_override: Option<&str>) -> Result<String, String> {
    let raw = std::fs::read_to_string(machine_path)
        .map_err(|e| format!("read {}: {e}", machine_path.display()))?;
    let source = crate::declarative::region::parse_source_with_limits(
        &raw,
        crate::declarative::region::RegionLimits::default(),
    )
    .map_err(|e| format!("parse {}: {}", machine_path.display(), e.join("; ")))?;
    let compiled = crate::declarative::compile::compile(source.definition)
        .map_err(|e| format!("compile {}: {}", machine_path.display(), e.join("; ")))?;
    let def = compiled.definition();
    let table = table_override
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}s", def.machine_id));

    let mut states: Vec<String> = def
        .states
        .iter()
        .filter(|(_, s)| !s.initial)
        .map(|(k, _)| k.clone())
        .collect();
    if states.is_empty() {
        states = def.states.keys().cloned().collect();
    }
    states.sort();

    let mut patterns = vec![
        format!("UPDATE {table} SET *status*"),
        format!("UPDATE {table} SET *state*"),
    ];
    for st in &states {
        patterns.push(format!(".status = \"{st}\""));
        patterns.push(format!(".status = '{st}'"));
        patterns.push(format!("status: \"{st}\""));
        patterns.push(format!("status: '{st}'"));
        patterns.push(format!(".Update(\"status\", \"{st}\")"));
    }

    let mut out = String::new();
    out.push_str(&format!(
        "# Generated by `ik guard init` from {} (machine `{}`).\n",
        machine_path.display(),
        def.machine_id
    ));
    out.push_str("# Exemptions belong in `allow_paths` here; inline ignore comments in source files are ignored.\n");
    out.push_str("guard_version = 1\n\n[[rules]]\n");
    out.push_str(&format!("id = \"{}-status-authority\"\n", def.machine_id));
    out.push_str(&format!(
        "description = \"Direct `{table}` status writes must go through the `{}` Desmo projection\"\n",
        def.machine_id
    ));
    out.push_str("case_insensitive = true\nforbidden_patterns = [\n");
    for (idx, pat) in patterns.iter().enumerate() {
        let comma = if idx + 1 < patterns.len() { "," } else { "" };
        out.push_str(&format!("  {}{comma}\n", serde_json::to_string(pat).unwrap()));
    }
    out.push_str("]\n");
    out.push_str("include_globs = [\"*.ts\", \"*.tsx\", \"*.js\", \"*.mjs\", \"*.py\", \"*.go\", \"*.sql\"]\n");
    out.push_str("allow_paths = [\"src/projections/*\", \"tests/*\"]\n");
    Ok(out)
}

fn run_init(args: &[String]) -> i32 {
    let mut out = PathBuf::from(DEFAULT_CONFIG);
    let mut machine: Option<PathBuf> = None;
    let mut table: Option<String> = None;
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--machine" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("ik guard init: --machine requires a path\n{USAGE}");
                    return 2;
                };
                machine = Some(PathBuf::from(val));
            }
            "--table" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("ik guard init: --table requires a name\n{USAGE}");
                    return 2;
                };
                table = Some(val.clone());
            }
            "--out" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("ik guard init: --out requires a path\n{USAGE}");
                    return 2;
                };
                out = PathBuf::from(val);
            }
            "--force" => force = true,
            flag if flag.starts_with('-') => {
                eprintln!("ik guard init: unknown flag `{flag}`\n{USAGE}");
                return 2;
            }
            other => {
                eprintln!("ik guard init: unexpected argument `{other}`\n{USAGE}");
                return 2;
            }
        }
        i += 1;
    }
    if out.exists() && !force {
        eprintln!(
            "ik guard init: {} already exists (pass --force to overwrite)",
            out.display()
        );
        return 2;
    }
    let body = match machine.as_deref() {
        Some(path) => match guard_toml_for_machine(path, table.as_deref()) {
            Ok(toml) => toml,
            Err(e) => {
                eprintln!("ik guard init: {e}");
                return 2;
            }
        },
        None => STARTER_GUARD_TOML.to_string(),
    };
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("ik guard init: create {}: {e}", parent.display());
            return 2;
        }
    }
    if let Err(e) = std::fs::write(&out, body) {
        eprintln!("ik guard init: write {}: {e}", out.display());
        return 2;
    }
    println!("Wrote {}", out.display());
    0
}

fn run_check(args: &[String]) -> i32 {
    let mut config = PathBuf::from(DEFAULT_CONFIG);
    let mut dir = PathBuf::from(".");
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--config" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("ik guard check: --config requires a path\n{USAGE}");
                    return 2;
                };
                config = PathBuf::from(val);
            }
            "--dir" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("ik guard check: --dir requires a path\n{USAGE}");
                    return 2;
                };
                dir = PathBuf::from(val);
            }
            "--json" => json_out = true,
            flag if flag.starts_with('-') => {
                eprintln!("ik guard check: unknown flag `{flag}`\n{USAGE}");
                return 2;
            }
            other => {
                eprintln!("ik guard check: unexpected argument `{other}`\n{USAGE}");
                return 2;
            }
        }
        i += 1;
    }
    let cfg = match load_config(&config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("ik guard check: {e}");
            return 2;
        }
    };
    let report = match check_dir(&dir, &cfg, &config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ik guard check: {e}");
            return 2;
        }
    };
    if json_out {
        let payload = json!({
            "schema": SCHEMA,
            "ok": report.ok,
            "rules_checked": report.rules_checked,
            "files_scanned": report.files_scanned,
            "violations": report.violations,
        });
        println!("{}", canonical_json(&payload));
    } else if report.ok {
        println!(
            "ik guard: OK ({} rules checked across {} files, 0 violations)",
            report.rules_checked, report.files_scanned
        );
    } else {
        println!(
            "ik guard: FAILED ({} violation(s) across {} files, {} rules checked):",
            report.violations.len(),
            report.files_scanned,
            report.rules_checked
        );
        for v in &report.violations {
            println!(
                "  {}:{} [{}] matched `{}` -> {}",
                v.file, v.line, v.rule_id, v.pattern, v.snippet
            );
        }
    }
    if report.ok { 0 } else { 1 }
}
