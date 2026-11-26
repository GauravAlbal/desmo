//! Standalone quickstart subcommands baked directly into `ik`:
//! - `ik demo [orders|stale-worker] [--json]`
//! - `ik init [order|job] --lang ts|py|go [--out <dir>] [--force]`
//! - `ik doctor [--dir <path>] [--json]`
//!
//! Works immediately after `install.sh` with zero Node/Python/Go prerequisites.

use crate::canonical::canonical_json;
use crate::codegen::{self, TargetLang};
use crate::declarative::{load_def, smart_starter_corpus, structural_lints};
use crate::identity::AggregateId;
use crate::machine::Machine;
use crate::protocol::{PROTOCOL_MAJOR, RUNTIME_VERSION};
use crate::store::{CommandRequest, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SCHEMA_DIRECTIVE: &str =
    "#:schema https://raw.githubusercontent.com/GauravAlbal/desmo/main/schema/ik.schema.json\n";
const ORDER_TOML: &str = include_str!("../examples/orders/order.ik.toml");
const JOB_TOML: &str = include_str!("../machines/verification_attempt.ik.toml");

const DEMO_USAGE: &str = "usage: ik demo [orders|stale-worker] [--json]";
const INIT_USAGE: &str =
    "usage: ik init [order|job] --lang ts|py|go [--out <dir>] [--force]";
const DOCTOR_USAGE: &str = "usage: ik doctor [--dir <path>] [--json]";

pub fn run_demo(args: &[String]) -> i32 {
    match run_demo_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik demo: {msg}");
            eprintln!("{DEMO_USAGE}");
            2
        }
    }
}

fn run_demo_inner(args: &[String]) -> Result<i32, String> {
    let mut target: Option<&str> = None;
    let mut json_out = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{DEMO_USAGE}");
                return Ok(0);
            }
            "--json" => json_out = true,
            "orders" | "stale-worker" if target.is_none() => target = Some(arg.as_str()),
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other => {
                return Err(format!(
                    "unknown demo `{other}` (expected `orders` or `stale-worker`)"
                ))
            }
        }
    }
    let run_orders = matches!(target, None | Some("orders"));
    let run_stale = matches!(target, None | Some("stale-worker"));

    let mut report = serde_json::Map::new();
    if run_orders {
        let res = demo_orders(json_out)?;
        report.insert("orders".into(), res);
    }
    if run_stale {
        if run_orders && !json_out {
            println!();
        }
        let res = demo_stale_worker(json_out)?;
        report.insert("stale_worker".into(), res);
    }
    if json_out {
        println!("{}", canonical_json(&Value::Object(report)));
    } else {
        println!("\nNext: scaffold a runnable project in your language:");
        println!("  ik init order --lang ts   # or --lang py | --lang go");
    }
    Ok(0)
}

fn temp_store_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "desmo-demo-{tag}-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn cmd_req(
    aggregate_id: &str,
    machine: &crate::identity::MachineIdentity,
    capability: crate::identity::Capability,
    command: Value,
) -> CommandRequest {
    CommandRequest {
        aggregate_id: AggregateId(aggregate_id.to_string()),
        machine: machine.clone(),
        capability,
        command,
        observation: None,
    }
}

fn demo_orders(quiet: bool) -> Result<Value, String> {
    let machine = crate::declarative::parse_and_compile(ORDER_TOML)
        .map_err(|e| format!("compile order machine: {}", e.join("; ")))?;
    let id = machine.identity().clone();
    let db_path = temp_store_path("orders");
    let _cleanup = TempFileGuard(db_path.clone());
    let mut store = Store::open_or_create(&db_path).map_err(|e| e.message)?;

    let unpaid = store
        .create_aggregate(&machine, &AggregateId("order-1042".into()), &id)
        .map_err(|e| e.message)?;
    let unpaid_err = store
        .command(
            &machine,
            &cmd_req(
                "order-1042",
                &id,
                unpaid.capability.clone(),
                json!({ "name": "refund", "args": {} }),
            ),
        )
        .expect_err("unpaid refund must refuse");
    let unpaid_fresh = store
        .show(&AggregateId("order-1042".into()))
        .map_err(|e| e.message)?;

    let o2 = store
        .create_aggregate(&machine, &AggregateId("order-1043".into()), &id)
        .map_err(|e| e.message)?;
    let paid = store
        .command(
            &machine,
            &cmd_req(
                "order-1043",
                &id,
                o2.capability,
                json!({ "name": "pay", "args": { "payment_ref": "ch_1043" } }),
            ),
        )
        .map_err(|e| e.message)?;
    let cancelled = store
        .command(
            &machine,
            &cmd_req(
                "order-1043",
                &id,
                paid.capability,
                json!({ "name": "cancel", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    let deliver_err = store
        .command(
            &machine,
            &cmd_req(
                "order-1043",
                &id,
                cancelled.capability,
                json!({ "name": "deliver", "args": {} }),
            ),
        )
        .expect_err("cancelled delivery must refuse");
    let cancelled_fresh = store
        .show(&AggregateId("order-1043".into()))
        .map_err(|e| e.message)?;

    let o3 = store
        .create_aggregate(&machine, &AggregateId("order-1044".into()), &id)
        .map_err(|e| e.message)?;
    let s1 = store
        .command(
            &machine,
            &cmd_req(
                "order-1044",
                &id,
                o3.capability,
                json!({ "name": "pay", "args": { "payment_ref": "ch_1044" } }),
            ),
        )
        .map_err(|e| e.message)?;
    let s2 = store
        .command(
            &machine,
            &cmd_req(
                "order-1044",
                &id,
                s1.capability,
                json!({ "name": "deliver", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    let s3 = store
        .command(
            &machine,
            &cmd_req(
                "order-1044",
                &id,
                s2.capability,
                json!({ "name": "send_back", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    let s4 = store
        .command(
            &machine,
            &cmd_req(
                "order-1044",
                &id,
                s3.capability,
                json!({ "name": "refund", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    let double_refund_err = store
        .command(
            &machine,
            &cmd_req(
                "order-1044",
                &id,
                s4.capability.clone(),
                json!({ "name": "refund", "args": {} }),
            ),
        )
        .expect_err("double refund must refuse");

    if !quiet {
        println!("=== Demo: orders (illegal transitions refused at runtime) ===");
        println!("Without Desmo (direct status writes): unpaid cart -> 'refunded', cancelled -> 'delivered', rogue 'PAID'");
        println!("With Desmo (order.ik.toml enforced by ik + SQLite):");
        println!(
            "  refund(cart)       -> refused ({}) · stays {}",
            unpaid_err.code.as_str(),
            unpaid_fresh.state["state"].as_str().unwrap_or("")
        );
        println!(
            "  deliver(cancelled) -> refused ({}) · stays {}",
            deliver_err.code.as_str(),
            cancelled_fresh.state["state"].as_str().unwrap_or("")
        );
        println!(
            "  pay->deliver->send_back->refund -> committed ({}, gen {})",
            s4.state["state"].as_str().unwrap_or(""),
            s4.generation
        );
        println!(
            "  refund(refunded)   -> refused ({}) · terminal lock holds",
            double_refund_err.code.as_str()
        );
    }

    Ok(json!({
        "unpaid_refund_code": unpaid_err.code.as_str(),
        "unpaid_state": unpaid_fresh.state["state"],
        "cancelled_deliver_code": deliver_err.code.as_str(),
        "cancelled_state": cancelled_fresh.state["state"],
        "happy_path_state": s4.state["state"],
        "happy_path_generation": s4.generation,
        "double_refund_code": double_refund_err.code.as_str(),
    }))
}

fn demo_stale_worker(quiet: bool) -> Result<Value, String> {
    let machine = crate::declarative::parse_and_compile(JOB_TOML)
        .map_err(|e| format!("compile job machine: {}", e.join("; ")))?;
    let id = machine.identity().clone();
    let db_path = temp_store_path("stale");
    let _cleanup = TempFileGuard(db_path.clone());
    let mut store = Store::open_or_create(&db_path).map_err(|e| e.message)?;

    let created = store
        .create_aggregate(&machine, &AggregateId("job-1".into()), &id)
        .map_err(|e| e.message)?;
    let running = store
        .command(
            &machine,
            &cmd_req(
                "job-1",
                &id,
                created.capability,
                json!({ "name": "start", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    // Worker A keeps a copy of `running.capability` (generation 1) while Worker B cancels.
    let stale_cap = running.capability.clone();
    let cancelled = store
        .command(
            &machine,
            &cmd_req(
                "job-1",
                &id,
                running.capability,
                json!({ "name": "cancel", "args": {} }),
            ),
        )
        .map_err(|e| e.message)?;
    let stale_err = store
        .command(
            &machine,
            &cmd_req(
                "job-1",
                &id,
                stale_cap,
                json!({
                    "name": "submit",
                    "args": { "artifact_digest": "late", "baseline_digest": "worker" }
                }),
            ),
        )
        .expect_err("stale worker must refuse");
    let fresh = store
        .show(&AggregateId("job-1".into()))
        .map_err(|e| e.message)?;

    if !quiet {
        println!("=== Demo: stale-worker (generation-bound capability fencing) ===");
        println!("Without Desmo: Worker B cancels -> late Worker A overwrites state to 'submitted'");
        println!("With Desmo (verification_attempt.ik.toml enforced by ik + SQLite):");
        println!(
            "  Worker A starts (gen 1), Worker B cancels (gen {}, {})",
            cancelled.generation,
            cancelled.state["state"].as_str().unwrap_or("")
        );
        println!(
            "  Late Worker A submit(gen 1) -> refused ({}) · canonical state stays {} (gen {})",
            stale_err.code.as_str(),
            fresh.state["state"].as_str().unwrap_or(""),
            fresh.generation
        );
    }

    Ok(json!({
        "cancelled_generation": cancelled.generation,
        "stale_refusal_code": stale_err.code.as_str(),
        "canonical_state": fresh.state["state"],
        "canonical_generation": fresh.generation,
    }))
}

pub fn run_init(args: &[String]) -> i32 {
    match run_init_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik init: {msg}");
            eprintln!("{INIT_USAGE}");
            2
        }
    }
}

fn run_init_inner(args: &[String]) -> Result<i32, String> {
    let mut template: Option<&str> = None;
    let mut lang: Option<TargetLang> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{INIT_USAGE}");
                return Ok(0);
            }
            "--lang" => {
                i += 1;
                let raw = args.get(i).ok_or("--lang requires ts|py|go")?;
                lang = Some(TargetLang::parse(raw)?);
            }
            "--out" => {
                i += 1;
                let raw = args.get(i).ok_or("--out requires a directory path")?;
                out_dir = Some(PathBuf::from(raw));
            }
            "--force" => force = true,
            "order" | "job" if template.is_none() => template = Some(args[i].as_str()),
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other => {
                return Err(format!(
                    "unknown template `{other}` (expected `order` or `job`)"
                ))
            }
        }
        i += 1;
    }

    let template = template.unwrap_or("order");
    let lang = lang.ok_or("--lang ts|py|go is required")?;
    let out_dir = out_dir.unwrap_or_else(|| PathBuf::from(format!("desmo-{template}")));

    let (toml_name, toml_src) = match template {
        "order" => ("order.ik.toml", ORDER_TOML),
        "job" => ("job.ik.toml", JOB_TOML),
        _ => unreachable!(),
    };
    let compiled = crate::declarative::parse_and_compile(toml_src)
        .map_err(|e| format!("compile template: {}", e.join("; ")))?;
    let def = compiled.definition();
    let cases_json = serde_json::to_string_pretty(&smart_starter_corpus(def))
        .map_err(|e| e.to_string())?;
    let go_pkg = (lang == TargetLang::Go).then_some("main");
    let generated = codegen::generate(def, compiled.identity(), lang, None, go_pkg);
    let toml_out = if toml_src.starts_with("#:schema ") {
        toml_src.to_string()
    } else {
        format!("{SCHEMA_DIRECTIVE}{toml_src}")
    };

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let mut files: Vec<(String, String)> = vec![
        (toml_name.to_string(), toml_out),
        (
            format!("{template}.ik.cases.json"),
            format!("{cases_json}\n"),
        ),
    ];

    match lang {
        TargetLang::TypeScript => {
            files.push((format!("{template}.machine.ts"), generated));
            files.push(("demo.mjs".into(), ts_demo_script(template, toml_name)));
            files.push(("package.json".into(), ts_package_json(template)));
        }
        TargetLang::Python => {
            files.push((format!("{template}_machine.py"), generated));
            files.push(("demo.py".into(), py_demo_script(template, toml_name)));
        }
        TargetLang::Go => {
            files.push((format!("{template}_machine.go"), generated));
            files.push(("main.go".into(), go_demo_script(template, toml_name)));
            files.push(("go.mod".into(), go_mod_file(template)));
        }
    }

    if !force {
        for (rel, _) in &files {
            let p = out_dir.join(rel);
            if p.exists() {
                return Err(format!(
                    "{} already exists (pass --force to overwrite)",
                    p.display()
                ));
            }
        }
    }
    for (rel, content) in &files {
        let p = out_dir.join(rel);
        std::fs::write(&p, content).map_err(|e| format!("write {}: {e}", p.display()))?;
    }

    println!("Initialized Desmo `{template}` project in {}:", out_dir.display());
    for (rel, _) in &files {
        println!("  {}", out_dir.join(rel).display());
    }
    println!("\nNext steps:");
    println!("  ik diagram {}/{toml_name}", out_dir.display());
    println!("  ik check {}/{toml_name} --summary", out_dir.display());
    println!(
        "  ik simulate {}/{toml_name} --out {}/{template}-simulator.html --open",
        out_dir.display(),
        out_dir.display()
    );
    match lang {
        TargetLang::TypeScript => {
            let plat = match (std::env::consts::OS, std::env::consts::ARCH) {
                ("macos", "aarch64") => "darwin-arm64",
                _ => "linux-x64",
            };
            println!(
                "  cd {} && curl -fsSLO https://github.com/GauravAlbal/desmo/releases/download/v{RUNTIME_VERSION}/desmo-runtime-{RUNTIME_VERSION}.tgz \\",
                out_dir.display()
            );
            println!(
                "    && curl -fsSLO https://github.com/GauravAlbal/desmo/releases/download/v{RUNTIME_VERSION}/desmo-{plat}-{RUNTIME_VERSION}.tgz \\"
            );
            println!(
                "    && npm install --no-audit --no-fund --ignore-scripts ./desmo-runtime-{RUNTIME_VERSION}.tgz ./desmo-{plat}-{RUNTIME_VERSION}.tgz \\"
            );
            println!("    && node demo.mjs");
        }
        TargetLang::Python => println!(
            "  pip install https://github.com/GauravAlbal/desmo/releases/download/v{RUNTIME_VERSION}/invariant_kernel-{RUNTIME_VERSION}-py3-none-any.whl\n  python3 {}/demo.py",
            out_dir.display()
        ),
        TargetLang::Go => println!(
            "  cd {} && go get github.com/GauravAlbal/desmo/go@go/v{RUNTIME_VERSION} && go run .",
            out_dir.display()
        ),
    }
    Ok(0)
}

fn ts_package_json(template: &str) -> String {
    format!(
        r#"{{
  "name": "desmo-{template}-example",
  "private": true,
  "type": "module",
  "scripts": {{
    "demo": "node demo.mjs"
  }}
}}
"#
    )
}

fn ts_demo_script(template: &str, toml_name: &str) -> String {
    if template == "order" {
        format!(
            r#"import {{ join }} from "node:path";
import {{ fileURLToPath }} from "node:url";
import {{ Kernel, IllegalStateError }} from "@desmo/runtime";
// In TypeScript projects (via `tsc` or `tsx`), import OrderMachine from "./order.machine.ts".

const here = fileURLToPath(new URL(".", import.meta.url));
const kernel = new Kernel(join(here, ".state", "orders.db"));
try {{
  const orders = await kernel.machine(join(here, "{toml_name}"));
  try {{
    const order = await orders.create("order-1");
    try {{
      await order.command("refund");
    }} catch (err) {{
      if (err instanceof IllegalStateError) {{
        console.log("Refused illegal refund from cart:", err.code);
      }} else {{
        throw err;
      }}
    }}
    const paid = await order.command("pay", {{ payment_ref: "ch_001" }});
    console.log("Committed:", paid.namedState(), "gen", paid.generation.toString());
  }} catch (err) {{
    if (!(err instanceof IllegalStateError)) {{
      throw err;
    }}
    const existing = await orders.show("order-1");
    console.log("Loaded existing order-1:", existing.namedState(), "gen", existing.generation.toString());
    try {{
      await existing.command("pay", {{ payment_ref: "ch_002" }});
    }} catch (payErr) {{
      if (payErr instanceof IllegalStateError) {{
        console.log("Refused pay from", existing.namedState() + ":", payErr.code);
      }} else {{
        throw payErr;
      }}
    }}
  }}
}} finally {{
  kernel.close();
}}
"#
        )
    } else {
        format!(
            r#"import {{ join }} from "node:path";
import {{ fileURLToPath }} from "node:url";
import {{ Kernel, IllegalStateError, StaleCapabilityError }} from "@desmo/runtime";
// In TypeScript projects (via `tsc` or `tsx`), import VerificationAttemptMachine from "./job.machine.ts".

const here = fileURLToPath(new URL(".", import.meta.url));
const kernel = new Kernel(join(here, ".state", "jobs.db"));
try {{
  const jobs = await kernel.machine(join(here, "{toml_name}"));
  try {{
    const created = await jobs.create("job-1");
    const running = await created.command("start");
    const cancelled = await running.command("cancel");
    console.log("Cancelled:", cancelled.namedState(), "gen", cancelled.generation.toString());
    try {{
      await running.command("submit", {{ artifact_digest: "late", baseline_digest: "worker" }});
    }} catch (err) {{
      if (err instanceof StaleCapabilityError) {{
        console.log("Stale worker refused:", err.code);
      }} else {{
        throw err;
      }}
    }}
  }} catch (err) {{
    if (!(err instanceof IllegalStateError)) {{
      throw err;
    }}
    const existing = await jobs.show("job-1");
    console.log("Loaded existing job-1:", existing.namedState(), "gen", existing.generation.toString());
    try {{
      await existing.command("submit", {{ artifact_digest: "late", baseline_digest: "worker" }});
    }} catch (subErr) {{
      if (subErr instanceof IllegalStateError) {{
        console.log("Refused submit from", existing.namedState() + ":", subErr.code);
      }} else {{
        throw subErr;
      }}
    }}
  }}
}} finally {{
  kernel.close();
}}
"#
        )
    }
}

fn py_demo_script(template: &str, toml_name: &str) -> String {
    if template == "order" {
        format!(
            r#"from pathlib import Path
from invariant_kernel import Kernel, IllegalStateError
from order_machine import OrderMachine

here = Path(__file__).resolve().parent
with Kernel(store=here / ".state" / "orders.db", session=True) as kernel:
    orders = OrderMachine(kernel.machine(here / "{toml_name}"))
    try:
        order = orders.create("order-1")
    except IllegalStateError:
        existing = orders.show("order-1")
        print("Loaded existing order-1:", existing.state_name, "gen", existing.generation)
        try:
            existing.pay(payment_ref="ch_002")
        except IllegalStateError as err:
            print(f"Refused pay from {{existing.state_name}}:", err.code)
    else:
        try:
            order.refund()
        except IllegalStateError as err:
            print("Refused illegal refund from cart:", err.code)

        paid = order.pay(payment_ref="ch_001")
        print("Committed:", paid.state_name if hasattr(paid, "state_name") else "historical", "gen", paid.generation)
"#
        )
    } else {
        format!(
            r#"from pathlib import Path
from invariant_kernel import Kernel, HistoricalResult, IllegalStateError, StaleCapabilityError
from job_machine import VerificationAttemptMachine

here = Path(__file__).resolve().parent
with Kernel(store=here / ".state" / "jobs.db", session=True) as kernel:
    jobs = VerificationAttemptMachine(kernel.machine(here / "{toml_name}"))
    try:
        created = jobs.create("job-1")
    except IllegalStateError:
        existing = jobs.show("job-1")
        print("Loaded existing job-1:", existing.state_name, "gen", existing.generation)
        try:
            existing.submit(artifact_digest="late", baseline_digest="worker")
        except IllegalStateError as err:
            print(f"Refused submit from {{existing.state_name}}:", err.code)
    else:
        running = created.start()
        assert not isinstance(running, HistoricalResult)
        cancelled = running.cancel()
        assert not isinstance(cancelled, HistoricalResult)
        print("Cancelled:", cancelled.state_name, "gen", cancelled.generation)
        try:
            running.submit(artifact_digest="late", baseline_digest="worker")
        except StaleCapabilityError as err:
            print("Stale worker refused:", err.code)
"#
        )
    }
}

fn go_mod_file(template: &str) -> String {
    format!(
        r#"module desmo-{template}-example

go 1.22

require github.com/GauravAlbal/desmo/go v{RUNTIME_VERSION}
"#
    )
}

fn go_demo_script(template: &str, toml_name: &str) -> String {
    if template == "order" {
        format!(
            r#"package main

import (
	"context"
	"errors"
	"fmt"
	"log"

	ik "github.com/GauravAlbal/desmo/go"
)

func main() {{
	ctx := context.Background()
	k, err := ik.Open(".state/orders.db", ik.Options{{Session: true}})
	if err != nil {{
		log.Fatal(err)
	}}
	defer k.Close()

	rawMachine, err := k.Machine(ctx, "{toml_name}")
	if err != nil {{
		log.Fatal(err)
	}}
	orders, err := WrapOrderMachine(rawMachine)
	if err != nil {{
		log.Fatal(err)
	}}
	order, err := orders.Create(ctx, "order-1")
	if err != nil {{
		var illegal *ik.IllegalStateError
		if !errors.As(err, &illegal) {{
			log.Fatal(err)
		}}
		existing, showErr := orders.Show(ctx, "order-1")
		if showErr != nil {{
			log.Fatal(showErr)
		}}
		fmt.Printf("Loaded existing order-1: %s (gen %d)\n", existing.StateName(), existing.Raw.Generation)
		if _, _, payErr := existing.Pay(ctx, "demo-pay-2", OrderPayArgs{{PaymentRef: "ch_002"}}); payErr != nil {{
			fmt.Printf("Refused pay from %s: %v\n", existing.StateName(), payErr)
		}}
		return
	}}
	if _, _, err := order.Refund(ctx, "demo-refund-check"); err != nil {{
		fmt.Println("Refused illegal refund from cart:", err)
	}}
	paid, _, err := order.Pay(ctx, "demo-pay-1", OrderPayArgs{{PaymentRef: "ch_001"}})
	if err != nil {{
		log.Fatal(err)
	}}
	fmt.Printf("Committed: %s (gen %d)\n", paid.StateName(), paid.Raw.Generation)
}}
"#
        )
    } else {
        format!(
            r#"package main

import (
	"context"
	"errors"
	"fmt"
	"log"

	ik "github.com/GauravAlbal/desmo/go"
)

func main() {{
	ctx := context.Background()
	k, err := ik.Open(".state/jobs.db", ik.Options{{Session: true}})
	if err != nil {{
		log.Fatal(err)
	}}
	defer k.Close()

	rawMachine, err := k.Machine(ctx, "{toml_name}")
	if err != nil {{
		log.Fatal(err)
	}}
	jobs, err := WrapVerificationAttemptMachine(rawMachine)
	if err != nil {{
		log.Fatal(err)
	}}
	created, err := jobs.Create(ctx, "job-1")
	if err != nil {{
		var illegal *ik.IllegalStateError
		if !errors.As(err, &illegal) {{
			log.Fatal(err)
		}}
		existing, showErr := jobs.Show(ctx, "job-1")
		if showErr != nil {{
			log.Fatal(showErr)
		}}
		fmt.Printf("Loaded existing job-1: %s (gen %d)\n", existing.StateName(), existing.Raw.Generation)
		if _, _, subErr := existing.Submit(ctx, "demo-submit-2", VerificationAttemptSubmitArgs{{
			ArtifactDigest: "late",
			BaselineDigest: "worker",
		}}); subErr != nil {{
			fmt.Printf("Refused submit from %s: %v\n", existing.StateName(), subErr)
		}}
		return
	}}
	running, _, err := created.Start(ctx, "demo-start-1")
	if err != nil {{
		log.Fatal(err)
	}}
	cancelled, _, err := running.Cancel(ctx, "demo-cancel-1")
	if err != nil {{
		log.Fatal(err)
	}}
	fmt.Printf("Cancelled: %s (gen %d)\n", cancelled.StateName(), cancelled.Raw.Generation)
	if _, _, err := running.Submit(ctx, "demo-stale-1", VerificationAttemptSubmitArgs{{
		ArtifactDigest: "late",
		BaselineDigest: "worker",
	}}); err != nil {{
		fmt.Println("Stale worker refused:", err)
	}}
}}
"#
        )
    }
}

pub fn run_doctor(args: &[String]) -> i32 {
    match run_doctor_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik doctor: {msg}");
            eprintln!("{DOCTOR_USAGE}");
            2
        }
    }
}

fn run_doctor_inner(args: &[String]) -> Result<i32, String> {
    let mut dir = PathBuf::from(".");
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{DOCTOR_USAGE}");
                return Ok(0);
            }
            "--dir" => {
                i += 1;
                let raw = args.get(i).ok_or("--dir requires a path")?;
                dir = PathBuf::from(raw);
            }
            "--json" => json_out = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }

    let mut machine_paths = Vec::new();
    collect_machines(&dir, 0, 4, &mut machine_paths);
    machine_paths.sort();

    let mut machine_results = Vec::new();
    let mut healthy = true;
    for path in &machine_paths {
        match load_def(path) {
            Ok(compiled) => {
                let lints = structural_lints(compiled.definition(), &compiled);
                machine_results.push(json!({
                    "path": path.display().to_string(),
                    "ok": true,
                    "machine_id": compiled.identity().machine_id,
                    "version": compiled.identity().version,
                    "lint_count": lints.len(),
                    "lints": lints,
                }));
            }
            Err(errors) => {
                healthy = false;
                machine_results.push(json!({
                    "path": path.display().to_string(),
                    "ok": false,
                    "errors": errors,
                }));
            }
        }
    }

    let sdk_checks = check_sdk_pins(&dir);
    for check in &sdk_checks {
        if check["drift"].as_bool() == Some(true) {
            healthy = false;
        }
    }

    let skill_installed = dir.join(".agents/skills/desmo/SKILL.md").is_file()
        || dir.join("skills/desmo/SKILL.md").is_file();

    let report = json!({
        "runtime_version": RUNTIME_VERSION,
        "protocol_major": PROTOCOL_MAJOR,
        "dir": dir.display().to_string(),
        "healthy": healthy,
        "skill_installed": skill_installed,
        "machines": machine_results,
        "sdk_pins": sdk_checks,
    });

    if json_out {
        println!("{}", canonical_json(&report));
    } else {
        println!(
            "Desmo doctor: ik {RUNTIME_VERSION} (protocol {PROTOCOL_MAJOR}.0) · dir {}",
            dir.display()
        );
        println!(
            "  agent skill: {}",
            if skill_installed {
                "installed"
            } else {
                "not installed (optional: npx skills add GauravAlbal/desmo)"
            }
        );
        println!("  machines found: {}", machine_paths.len());
        for m in machine_results {
            if m["ok"].as_bool() == Some(true) {
                let lc = m["lint_count"].as_u64().unwrap_or(0);
                println!(
                    "    ok    {} ({} v{}, {} lints)",
                    m["path"].as_str().unwrap_or(""),
                    m["machine_id"].as_str().unwrap_or(""),
                    m["version"].as_u64().unwrap_or(0),
                    lc
                );
            } else {
                println!(
                    "    FAIL  {}: {:?}",
                    m["path"].as_str().unwrap_or(""),
                    m["errors"]
                );
            }
        }
        for s in sdk_checks {
            let status = if s["drift"].as_bool() == Some(true) {
                "DRIFT"
            } else {
                "ok   "
            };
            println!(
                "  sdk   {status} {} ({} vs runtime {RUNTIME_VERSION})",
                s["file"].as_str().unwrap_or(""),
                s["declared"].as_str().unwrap_or("")
            );
        }
        println!(
            "status: {}",
            if healthy { "healthy" } else { "issues found" }
        );
    }

    Ok(if healthy { 0 } else { 1 })
}

fn collect_machines(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if matches!(
                name,
                ".git" | "node_modules" | "target" | ".venv" | "venv" | "dist" | "__pycache__"
            ) {
                continue;
            }
            collect_machines(&path, depth + 1, max_depth, out);
        } else if name.ends_with(".ik.toml") {
            out.push(path);
        }
    }
}

fn check_sdk_pins(dir: &Path) -> Vec<Value> {
    let mut out = Vec::new();
    let pkg = dir.join("package.json");
    if let Ok(text) = std::fs::read_to_string(&pkg) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            let dep = v
                .get("dependencies")
                .and_then(|d| d.get("@desmo/runtime"))
                .or_else(|| v.get("devDependencies").and_then(|d| d.get("@desmo/runtime")))
                .and_then(Value::as_str);
            if let Some(declared) = dep {
                let drift = !declared.contains(RUNTIME_VERSION);
                out.push(json!({
                    "file": pkg.display().to_string(),
                    "sdk": "@desmo/runtime",
                    "declared": declared,
                    "drift": drift,
                }));
            }
        }
    }
    let gomod = dir.join("go.mod");
    if let Ok(text) = std::fs::read_to_string(&gomod) {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("replace ") || trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("GauravAlbal/desmo/go") {
                let declared = trimmed.to_string();
                let drift = !declared.contains(RUNTIME_VERSION);
                out.push(json!({
                    "file": gomod.display().to_string(),
                    "sdk": "github.com/GauravAlbal/desmo/go",
                    "declared": declared,
                    "drift": drift,
                }));
            }
        }
    }
    out
}

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let wal = PathBuf::from(format!("{}-wal", self.0.display()));
        let shm = PathBuf::from(format!("{}-shm", self.0.display()));
        let _ = std::fs::remove_file(wal);
        let _ = std::fs::remove_file(shm);
    }
}
