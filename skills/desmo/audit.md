# Audit: find implicit state machines and lifecycle bugs in an existing codebase

Use this mode when the user wants to find hidden state machines, review a flow for state bugs, or inspect an existing codebase **before** committing to adopting Desmo.

This mode runs strictly inside `investigate_only` (`read` + `scratch` classes in `routing.json` beside this file): **never edit application source and never touch a production store or database.**

## 1. Scan for implicit state machines

Search the repository (or the specific flow the user named) for lifecycle fields that behave as state machines without an authoritative boundary:

- String status columns or enums (`status`, `state`, `phase`, `stage`, `workflow_state`)
- Coupled boolean flag clusters (`isLoading`/`isSubmitting`/`approved`/`cancelled`/`archived`)
- Nullable milestone timestamps (`paid_at`, `approved_at`, `shipped_at`, `cancelled_at`)
- Async workers, queues, retries, and webhook handlers that update status asynchronously

Do not ask the human to locate writers — find every call site that reads or writes those fields yourself and record exact `file:line` coordinates.

## 2. Select the highest-risk lifecycle (or the user's named flow)

If the user named a flow (for example, checkout, upload, refunds, or publishing), audit that flow first. Otherwise rank candidates by blast radius (money movement, irreversible external effects, approval gates, terminal states like `cancelled` or `refunded` that async workers can overwrite) and pick the top one to three lifecycles.

For each audited lifecycle, list:
- Every declared or observed state value (including case drift like `"PAID"` vs `"paid"`, `null`, or ad-hoc states)
- Every writer (`file:line`) and which precondition (if any) it checks before writing
- Every unspoken business invariant the codebase assumes (for example: *"cancelled stays cancelled"*, *"edit invalidates approval"*, *"only refunded after returned"*)

## 3. Extract a scratch `.ik.toml` model and run the workbench

Write a draft declarative model in a fresh temporary directory (for example `/tmp/desmo-audit/<name>.ik.toml` — never cluttering or overwriting files inside the user's repo).

### Minimal `.ik.toml` syntax reference
```toml
#:schema https://raw.githubusercontent.com/GauravAlbal/desmo/main/schema/ik.schema.json
schema_version = 1
machine_id = "order"
machine_version = 1

[facts] # optional if no facts; types: "string" | "int" | "bool"; policy: "replace" | "counter" | "once"
payment_ref = { type = "string", policy = "replace" }
retries = { type = "int", policy = "counter", initial = 0 }
cleared = { type = "bool", policy = "replace", initial = false }

[states]
cart = { initial = true }
paid = {}
returned = {}
refunded = { terminal = true }
cancelled = { terminal = true }

[commands.pay]
from = ["cart"]
to = "paid"
args = { payment_ref = "string" }
when = "text_in($payment_ref, 'A-Za-z0-9._-', 1, 64)" # $prefix for args, bare name for facts
then = { payment_ref = "$payment_ref", cleared = true } # native bool/int & checked i64 math (e.g. "retries + 1") supported

[commands.refund]
from = ["returned"]
to = "refunded"
```

Run the read-only / scratch workbench commands against that scratch model:

```text
ik identity <machine.ik.toml>
ik diagram <machine.ik.toml>
ik check <machine.ik.toml> --auto-seed --summary
ik laws <machine.ik.toml>
ik simulate <machine.ik.toml> --out <simulator.html>
```

*(`ik check` with `--auto-seed` requires zero hand-written corpus and catches structural traps plus invariant violations right away; use `ik corpus init` + `--corpus` when testing custom business witness cases.)*

If `ik` is not on `PATH` yet, tell the user it can be installed in one command from the official release (`curl -fsSL https://raw.githubusercontent.com/GauravAlbal/desmo/main/install.sh | sh`) before running the workbench and generating the interactive simulator.

## 4. Check existing writers against four concrete bug classes

Compare every real writer (`file:line`) from step 1 against the extracted `.ik.toml` rules:

1. **Illegal-transition / skipped-state writes:** a handler writes a target status (such as `refunded` or `published`) without checking the required source state or invariant.
2. **Stale-worker / lost-update races:** a background worker, webhook, or retry overwrites a terminal or newer state (`cancelled → delivered`) because there is no generation/version capability check.
3. **Undeclared / rogue values:** string literals, case drift (`"PAID"`), `null` defaults, or cleanup jobs write states outside the lifecycle.
4. **Uncertain-effect / duplicate-retry gaps:** retries or timeouts after external calls lack an idempotent operation id or effect-outcome binding.

Cite only observed code paths with exact `file:line` references. Do not invent hypothetical bugs that the code does not exhibit.

## 5. Deliver the audit readout

Present a concise report to the user containing:

1. **Implicit lifecycles found** (ranked table: lifecycle, writers count, highest-risk bug class, `file:line` pointers).
2. **Structural diagram** (verbatim `ik diagram` output for the primary audited lifecycle).
3. **Concrete findings** (each citing exact `file:line`, the violating sequence, and what invariant breaks).
4. **Interactive simulator path** (`<scratch>/<name>-simulator.html` when generated — clearly labeled as an interactive scratch simulator, not runtime authority).
5. **Honest recommendation:**
   - **Keep in host** if the flow is simple CRUD or has no hard shared invariant.
   - **Harden behind Desmo** if illegal transitions, stale workers, or crash recovery matter — offer to switch to `build.md` (beside this file) to lock this exact `.ik.toml` behind `ik` (one command family first, projecting read-only via `ik events`, and adding `ik guard init --machine <machine.ik.toml> --table <table>` to prevent bypass writes once Desmo owns the transition).

State plainly that an audit examines the extracted model and observed call sites; it is not a proof of the entire application and modifies no source or production data.
