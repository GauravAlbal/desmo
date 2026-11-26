//! Durable SQLite runtime (K1/K2.5).
//!
//! One shared store serves declarative and native machines: every mutation runs inside
//! a `BEGIN IMMEDIATE` transaction in which capability validation, deterministic
//! reduction, journal append, projection update, generation increment, nonce rotation,
//! effect identity, and receipt commit are atomic. Same-generation races serialize and
//! the loser observes a stale capability. Reopen, replay, and verification are
//! supported, with projection/journal corruption detection.

use crate::canonical::{canonical_json, digest_json};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectOutcomeV1, EffectStatus};
use crate::identity::{
    AggregateId, Capability, MachineIdentity, MachineSpecHash, MachineVersion, StoreId,
};
#[cfg(feature = "test-support")]
use crate::machine::Decision;
use crate::machine::Machine;
use crate::operation::{
    Binding, Lookup, OperationHistory, OperationId, OperationKind, OperationOutcome,
    OperationRecord, OperationRequest,
};
use crate::protocol::{PROTOCOL_MAJOR, RUNTIME_VERSION};
use crate::runtime::{
    self, AggregateSnapshot, EffectDelta, JournalEvent, Receipt, Step, GENESIS_PREV,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

/// Persisted schema version. Version 4 adds the `operations` table (caller
/// operation attribution, `docs/operation-identity.md`) to the version 3 layout,
/// whose journal compacts domain events plus a hash-protected state digest; full
/// state exists only in the current projection. Stores are forward-only: a
/// version 3 store is migrated in place on open by adding an empty `operations`
/// table (no historical ids are invented); an older binary refuses a version 4
/// store. There is no in-place migration from schema 2.
pub const SCHEMA_VERSION: u64 = 4;

/// Schema version that `Store::open` upgrades in place.
const PREVIOUS_SCHEMA_VERSION: u64 = 3;

const OPERATIONS_TABLE: &str = "
CREATE TABLE operations (
  aggregate_id TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  first_sequence INTEGER NOT NULL,
  last_sequence INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  state_digest TEXT NOT NULL,
  event_hashes TEXT NOT NULL,
  receipt_ids TEXT NOT NULL,
  PRIMARY KEY (aggregate_id, operation_id)
);
";

const SCHEMA: &str = "
CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE aggregates (
  aggregate_id TEXT PRIMARY KEY,
  machine_id TEXT NOT NULL,
  machine_version INTEGER NOT NULL,
  machine_spec_hash TEXT NOT NULL,
  state TEXT NOT NULL,
  terminal INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  nonce TEXT NOT NULL,
  effect_json TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  last_event_hash TEXT NOT NULL,
  last_receipt_id TEXT NOT NULL
);
CREATE TABLE journal (
  aggregate_id TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  hash TEXT NOT NULL,
  prev_hash TEXT NOT NULL,
  event_json TEXT NOT NULL,
  PRIMARY KEY (aggregate_id, sequence)
);
CREATE TABLE receipts (
  aggregate_id TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  receipt_id TEXT NOT NULL,
  receipt_json TEXT NOT NULL,
  PRIMARY KEY (aggregate_id, sequence),
  UNIQUE (aggregate_id, receipt_id)
);
CREATE INDEX IF NOT EXISTS idx_aggregates_machine_id
  ON aggregates (machine_id, aggregate_id);
CREATE INDEX IF NOT EXISTS idx_aggregates_machine_terminal_id
  ON aggregates (machine_id, terminal, aggregate_id);
";

/// A mutating request for a domain command.
#[derive(Clone, Debug)]
pub struct CommandRequest {
    pub aggregate_id: AggregateId,
    pub machine: MachineIdentity,
    pub capability: Capability,
    pub command: Value,
    pub observation: Option<Value>,
}

/// An ordered, all-or-nothing batch of commands applied under ONE capability. The kernel threads
/// each committed step's successor capability internally; callers never supply per-step
/// capabilities, so a transaction cannot be used to forge or replay authority.
pub struct TransactionRequest {
    pub aggregate_id: AggregateId,
    pub machine: MachineIdentity,
    pub capability: Capability,
    pub steps: Vec<crate::protocol::TransactionStep>,
}

fn in_step(mut diagnostic: Diagnostic, index: usize) -> Diagnostic {
    diagnostic.message = format!("transaction step {index}: {}", diagnostic.message);
    diagnostic
}

/// A mutating request binding an effect outcome (kernel `record` operation).
#[derive(Clone, Debug)]
pub struct EffectOutcomeRequest {
    pub aggregate_id: AggregateId,
    pub machine: MachineIdentity,
    pub capability: Capability,
    pub effect: EffectOutcomeV1,
}

/// Result of the atomic create-with-initial-command operation.
///
/// Genesis and the initial command either commit together in ONE SQLite
/// transaction (IK-LAW-004) or not at all. A retry that would reproduce EXACTLY
/// the already-committed genesis + initial event returns [`CreateWithCommandOutcome::ExactDuplicate`]
/// with zero mutation and the original stored receipt; a divergent retry refuses
/// (PBT-005/PBT-006 analog: exact duplicate is idempotent, conflicting duplicate
/// refuses).
#[derive(Clone, Debug)]
pub enum CreateWithCommandOutcome {
    /// Genesis + the initial command event committed atomically in this transaction.
    Committed(CommandResult),
    /// Exact duplicate of an already-committed create-with-command: nothing was
    /// written; the stored projection and the original admission receipt are
    /// returned. Divergent admissions are never accepted.
    ExactDuplicate(CommandResult),
}

/// Result of a committed, replayed, or no-op mutation.
#[derive(Clone, Debug)]
pub struct CommandResult {
    pub receipt: Option<Receipt>,
    pub state: Value,
    pub terminal: bool,
    pub generation: u64,
    pub capability: Capability,
    pub effect: EffectStatus,
    pub mutation: bool,
    pub note: Option<Value>,
}

/// Read-only view of an aggregate.
#[derive(Clone, Debug, Serialize)]
pub struct Projection {
    pub aggregate_id: AggregateId,
    pub machine: MachineIdentity,
    pub state: Value,
    pub terminal: bool,
    pub generation: u64,
    pub capability: Capability,
    pub effect: EffectStatus,
    pub sequence: u64,
    pub last_event_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_receipt: Option<Receipt>,
}

/// Result of a replay: each aggregate's rebuilt projection.
#[derive(Clone, Debug, Serialize)]
pub struct ReplaySummary {
    pub store_id: StoreId,
    pub aggregates: Vec<ReplayEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayEntry {
    pub aggregate_id: AggregateId,
    pub sequence: u64,
    pub generation: u64,
    pub state: Value,
    pub rebuilt: bool,
}

/// Result of a verification pass.
#[derive(Clone, Debug, Serialize)]
pub struct VerifySummary {
    pub store_id: StoreId,
    pub aggregates: usize,
    pub journal_ok: bool,
    pub projection_ok: bool,
    pub verified: bool,
}

/// Test-only crash-injection hook invoked with each deterministic persistence
/// boundary until it returns `true`, which consumes the hook. Installed only
/// through [`Store::open_with_crash_hook`], which no production path calls; a
/// hook that terminates the process leaves the transaction uncommitted,
/// exercising SQLite crash recovery. A nonmatching hook returns `false` so a
/// later boundary in the same operation can be selected. A matching hook is
/// consumed and dropped with its store (RAII-cleared), so it cannot fire twice,
/// outlive its store, or affect another store. There is no environment or
/// configuration trigger: arming requires the dedicated test-only constructor.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub type CrashHook = Box<dyn FnMut(&str) -> bool>;

/// SQLite-backed runtime store. Not `Sync`; one connection per store.
pub struct Store {
    conn: Connection,
    store_id: StoreId,
    #[cfg(feature = "test-support")]
    crash_hook: Option<CrashHook>,
}

fn map_sqlite(err: rusqlite::Error) -> Diagnostic {
    if let rusqlite::Error::SqliteFailure(e, _) = &err {
        if e.code == rusqlite::ErrorCode::DatabaseBusy {
            return Diagnostic::new(
                DiagnosticCode::IkConcurrentMutation,
                crate::diagnostic::NO_RULE,
                format!("store busy: {err}"),
            );
        }
    }
    Diagnostic::new(
        DiagnosticCode::IkStoreCorrupt,
        crate::diagnostic::NO_RULE,
        format!("store failure: {err}"),
    )
}

fn sqlite_synchronous_pragma() -> &'static str {
    match std::env::var("IK_SQLITE_SYNCHRONOUS")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("normal") => "NORMAL",
        _ => "FULL",
    }
}

fn configure(conn: &Connection, is_create: bool) -> Result<(), Diagnostic> {
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(map_sqlite)?;
    let sync = sqlite_synchronous_pragma();
    if is_create {
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous={sync}; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-16000;"
        ))
        .map_err(map_sqlite)?;
    } else {
        conn.execute_batch(&format!(
            "PRAGMA synchronous={sync}; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-16000;"
        ))
        .map_err(map_sqlite)?;
    }
    Ok(())
}

fn meta_value(conn: &Connection, key: &str) -> Result<String, Diagnostic> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sqlite)?
    .ok_or_else(|| {
        Diagnostic::new(
            DiagnosticCode::IkStoreCorrupt,
            crate::diagnostic::NO_RULE,
            format!("store meta key `{key}` missing"),
        )
    })
}

/// Upgrade a version 3 store to the current schema in place: add an empty
/// `operations` table and advance `schema_version`, atomically. The version is
/// re-read inside the write transaction so two processes opening the same store
/// cannot both migrate it. Existing journal events receive no operation ids.
fn migrate_previous_schema(conn: &mut Connection) -> Result<(), Diagnostic> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite)?;
    if meta_value(&tx, "schema_version")? == PREVIOUS_SCHEMA_VERSION.to_string() {
        tx.execute_batch(OPERATIONS_TABLE).map_err(map_sqlite)?;
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .map_err(map_sqlite)?;
    }
    tx.commit().map_err(map_sqlite)
}

impl Store {
    /// Create a fresh store at `path`. Creates missing parent directories and refuses an existing path.
    pub fn create(path: &Path) -> Result<Store, Diagnostic> {
        if path.exists() {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("store.already_exists"),
                format!("store already exists at {}", path.display()),
            ));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Diagnostic::new(
                        DiagnosticCode::IkInternal,
                        Some("store.create_parent"),
                        format!("cannot create parent directory {}: {e}", parent.display()),
                    )
                })?;
            }
        }
        let mut conn = Connection::open(path).map_err(|e| {
            let diag = map_sqlite(e);
            if diag.code == DiagnosticCode::IkStoreCorrupt {
                Diagnostic::new(
                    DiagnosticCode::IkInternal,
                    Some("store.open_failed"),
                    diag.message,
                )
            } else {
                diag
            }
        })?;
        configure(&conn, true)?;
        let store_id = StoreId(format!("ik-store-{}", Uuid::new_v4()));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        tx.execute_batch(SCHEMA).map_err(map_sqlite)?;
        tx.execute_batch(OPERATIONS_TABLE).map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )
        .map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('store_id', ?1)",
            params![store_id.0],
        )
        .map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('runtime_version', ?1)",
            params![RUNTIME_VERSION],
        )
        .map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('protocol_major', ?1)",
            params![PROTOCOL_MAJOR.to_string()],
        )
        .map_err(map_sqlite)?;
        tx.commit().map_err(map_sqlite)?;
        Ok(Store {
            conn,
            store_id,
            #[cfg(feature = "test-support")]
            crash_hook: None,
        })
    }

    /// Open an existing store, validating the persisted schema version.
    ///
    /// A missing file is [`DiagnosticCode::IkStoreNotFound`] (`store.missing`).
    /// This method never creates the file or its parent directory. [`Store::create`]
    /// is the only constructor that creates a store file.
    pub fn open(path: &Path) -> Result<Store, Diagnostic> {
        if !path.exists() {
            return Err(Diagnostic::new(
                DiagnosticCode::IkStoreNotFound,
                Some("store.missing"),
                format!("store file missing at {}", path.display()),
            ));
        }
        let mut conn = Connection::open(path).map_err(map_sqlite)?;
        configure(&conn, false)?;
        let mut schema = meta_value(&conn, "schema_version")?;
        if schema == PREVIOUS_SCHEMA_VERSION.to_string() {
            migrate_previous_schema(&mut conn)?;
            schema = meta_value(&conn, "schema_version")?;
        }
        if schema != SCHEMA_VERSION.to_string() {
            return Err(Diagnostic::new(
                DiagnosticCode::IkPersistedSchemaUnknown,
                Some("store.schema_version"),
                format!("unknown persisted schema version `{schema}`"),
            ));
        }
        let store_id = StoreId(meta_value(&conn, "store_id")?);
        Ok(Store {
            conn,
            store_id,
            #[cfg(feature = "test-support")]
            crash_hook: None,
        })
    }

    /// Open an existing store or create a fresh one at `path` (race-safe under concurrent opens).
    pub fn open_or_create(path: &Path) -> Result<Store, Diagnostic> {
        match Store::open(path) {
            Ok(store) => Ok(store),
            Err(err) if err.code == DiagnosticCode::IkStoreNotFound => match Store::create(path) {
                Ok(store) => Ok(store),
                Err(create_err)
                    if create_err.code == DiagnosticCode::IkIllegalState
                        || create_err.message.contains("already exists") =>
                {
                    Store::open(path)
                }
                Err(create_err) => Err(create_err),
            },
            Err(err) => Err(err),
        }
    }

    /// Test-only: open an existing store with an explicitly injected crash hook.
    /// Used exclusively by the crash-matrix integration-test child, which arms
    /// exactly one deterministic boundary name and terminates the process when it
    /// fires. Never armed by production code; see [`CrashHook`].
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn open_with_crash_hook(path: &Path, crash_hook: CrashHook) -> Result<Store, Diagnostic> {
        let mut store = Store::open(path)?;
        store.crash_hook = Some(crash_hook);
        Ok(store)
    }

    pub fn store_id(&self) -> &StoreId {
        &self.store_id
    }

    /// Create a new aggregate (genesis event, generation 0) under `machine`. The
    /// presented machine identity must exactly match the loaded machine's identity.
    pub fn create_aggregate(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: &AggregateId,
        presented: &MachineIdentity,
    ) -> Result<CommandResult, Diagnostic> {
        check_create_identity(machine.identity(), presented)?;
        // Refuse an invalid genesis before the write transaction opens: no
        // aggregate, journal event, receipt, or capability may exist for it.
        validate_genesis(machine)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM aggregates WHERE aggregate_id = ?1",
                params![aggregate_id.0],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_sqlite)?;
        if exists.is_some() {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("create.duplicate_aggregate"),
                format!("aggregate `{}` already exists", aggregate_id.0),
            ));
        }
        let identity = machine.identity().clone();
        let initial = machine.initial_state();
        let create_command =
            crate::declarative::compile::create_command(&aggregate_id.0, &identity);
        let genesis = runtime::genesis_plan(&identity, aggregate_id, initial, create_command);
        let runtime::GenesisPlan {
            event,
            receipt,
            state,
            canonical_state,
        } = genesis;
        let generation: u64 = 0;
        let nonce = runtime::derive_nonce(&identity, aggregate_id, generation);
        persist_plan(
            &tx,
            aggregate_id,
            &event,
            &receipt,
            &EffectStatus::None,
            &canonical_state,
            &nonce,
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
            #[cfg(feature = "test-support")]
            "genesis",
        )?;
        tx.execute(
            "INSERT INTO aggregates (aggregate_id, machine_id, machine_version, machine_spec_hash, state, terminal, generation, nonce, effect_json, sequence, last_event_hash, last_receipt_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                aggregate_id.0,
                identity.machine_id,
                identity.version.0 as i64,
                identity.spec_hash.0,
                canonical_state,
                0i64,
                0i64,
                nonce,
                canonical_json(&EffectStatus::None),
                0i64,
                event.hash,
                receipt.receipt_id,
            ],
        )
        .map_err(map_sqlite)?;
        #[cfg(feature = "test-support")]
        fire_crash(
            &mut self.crash_hook,
            "crash.genesis.after_aggregates_insert",
        );
        tx.commit().map_err(map_sqlite)?;
        Ok(CommandResult {
            receipt: Some(receipt),
            state,
            terminal: false,
            generation: 0,
            capability: Capability {
                generation: 0,
                nonce,
            },
            effect: EffectStatus::None,
            mutation: true,
            note: None,
        })
    }

    /// Create a new aggregate AND apply its initial command in ONE atomic SQLite
    /// transaction: genesis at sequence 0, the initial command event at sequence 1
    /// when the command emits. A crash or error at any point leaves neither a
    /// partial aggregate nor a partial projection — journal, receipts, and the
    /// projection row commit together or not at all (IK-LAW-004, contract
    /// "aggregate genesis + admission is one SQLite transaction, one atomic public
    /// operation").
    ///
    /// Exact retry: a repeated request whose machine, aggregate id, command, and
    /// observation would reproduce EXACTLY the already-committed genesis and
    /// initial event returns [`CreateWithCommandOutcome::ExactDuplicate`] with zero
    /// mutation and the original stored receipt — the operation is idempotent and
    /// never accepts a divergent admission. Any retry that would have written
    /// different events refuses with the stable `create.command_conflict`
    /// diagnostic.
    pub fn create_aggregate_with_command(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: &AggregateId,
        presented: &MachineIdentity,
        command: &Value,
        observation: Option<&Value>,
    ) -> Result<CreateWithCommandOutcome, Diagnostic> {
        expect_committed(self.create_with_command_inner(
            machine,
            aggregate_id,
            presented,
            command,
            observation,
            None,
        )?)
    }

    /// [`Store::create_aggregate_with_command`] bound to a caller operation id. An
    /// exact repeat of an already committed operation returns
    /// [`OperationOutcome::Historical`] without running the reducer. The
    /// attribution row commits in the same transaction as genesis (and the first
    /// event when it commits).
    pub fn create_aggregate_with_command_op(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: &AggregateId,
        presented: &MachineIdentity,
        command: &Value,
        observation: Option<&Value>,
        operation_id: &OperationId,
    ) -> Result<OperationOutcome<CreateWithCommandOutcome>, Diagnostic> {
        self.create_with_command_inner(
            machine,
            aggregate_id,
            presented,
            command,
            observation,
            Some(operation_id),
        )
    }

    fn create_with_command_inner(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: &AggregateId,
        presented: &MachineIdentity,
        command: &Value,
        observation: Option<&Value>,
        operation_id: Option<&OperationId>,
    ) -> Result<OperationOutcome<CreateWithCommandOutcome>, Diagnostic> {
        check_create_identity(machine.identity(), presented)?;
        validate_genesis(machine)?;
        let binding = operation_id.map(|id| {
            Binding::new(
                id,
                &OperationRequest::CreateWithCommand {
                    machine: presented,
                    aggregate_id,
                    command,
                    observation,
                },
            )
        });
        // Exact repeat of a committed operation answers from its record, before
        // any reducer or worker runs.
        let probe = (&self.conn, machine, presented, aggregate_id);
        if let Some(history) = probe_recorded_history(probe, binding.as_ref())? {
            return Ok(OperationOutcome::Historical(history));
        }
        // Pure, deterministic construction: a refused command fails before any
        // write, so an error at this point leaves no partial state either.
        let (genesis_plan, step) =
            build_create_command_plan(machine, aggregate_id, command, observation)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let genesis = &genesis_plan.event;
        if let Some(existing) =
            existing_admission(&tx, aggregate_id, binding.as_ref(), genesis, &step)?
        {
            tx.commit().map_err(map_sqlite)?;
            return Ok(existing);
        }
        // Fresh path: genesis and the initial command event inside ONE transaction.
        let identity = machine.identity().clone();
        let nonce0 = runtime::derive_nonce(&identity, aggregate_id, 0);
        insert_genesis(
            &tx,
            aggregate_id,
            &identity,
            (&genesis_plan.event, &genesis_plan.receipt),
            &genesis_plan.canonical_state,
            &nonce0,
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
        )?;
        commit_admission(
            tx,
            aggregate_id,
            &identity,
            binding.as_ref(),
            (genesis_plan, nonce0),
            step,
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
        )
        .map(OperationOutcome::Committed)
    }

    /// Execute a domain command through the shared transaction path.
    pub fn command(
        &mut self,
        machine: &dyn Machine,
        req: &CommandRequest,
    ) -> Result<CommandResult, Diagnostic> {
        expect_committed(self.command_inner(machine, req, None)?)
    }

    /// [`Store::command`] bound to a caller operation id. An exact repeat of an
    /// already committed operation returns its recorded history before currentness
    /// is checked and without running the reducer; a different request under the
    /// same id refuses with `IK_OPERATION_CONFLICT`.
    pub fn command_op(
        &mut self,
        machine: &dyn Machine,
        req: &CommandRequest,
        operation_id: &OperationId,
    ) -> Result<OperationOutcome<CommandResult>, Diagnostic> {
        self.command_inner(machine, req, Some(operation_id))
    }

    fn command_inner(
        &mut self,
        machine: &dyn Machine,
        req: &CommandRequest,
        operation_id: Option<&OperationId>,
    ) -> Result<OperationOutcome<CommandResult>, Diagnostic> {
        let binding = operation_id.map(|id| Binding::new(id, &OperationRequest::Command(req)));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let snapshot = load_snapshot(&tx, &req.aggregate_id)?;
        check_machine_match(&snapshot, machine.identity(), &req.machine)?;
        if let Some(binding) = &binding {
            if let Some(history) = resolve_duplicate(&tx, &req.aggregate_id, binding)? {
                return Ok(OperationOutcome::Historical(history));
            }
        }
        let recorded_receipt = fetch_recorded_receipt(&tx, &snapshot)?;
        let step = match runtime::execute_command(
            machine,
            &snapshot,
            recorded_receipt.as_ref(),
            &req.capability,
            &req.command,
            req.observation.as_ref(),
        ) {
            Ok(step) => step,
            Err(err) => {
                #[cfg(feature = "test-support")]
                if crate::law_mutant::is(crate::law_mutant::LawMutant::PersistOnRefusal) {
                    let _ = tx.execute(
                        "UPDATE aggregates SET sequence = sequence + 1 WHERE aggregate_id = ?1",
                        rusqlite::params![req.aggregate_id.0],
                    );
                    let _ = tx.commit();
                }
                return Err(err);
            }
        };
        finish_step(
            tx,
            &snapshot,
            step,
            binding.as_ref(),
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
            #[cfg(feature = "test-support")]
            "command",
        )
        .map(OperationOutcome::Committed)
    }

    /// Execute an ordered list of commands as ONE all-or-nothing SQLite transaction. Each step
    /// runs the exact same capability check, decide, apply, journal, receipt, projection and
    /// nonce-rotation path as [`Store::command`] (CORE_LAWS-4 and -10); a failure at any step
    /// rolls back every prior step. Only the LAST step may create an effect intent: the host
    /// must not be asked to authorize external work and then keep reducing in the same batch.
    pub fn transaction(
        &mut self,
        machine: &dyn Machine,
        req: &TransactionRequest,
    ) -> Result<Vec<CommandResult>, Diagnostic> {
        expect_committed(self.transaction_inner(machine, req, None)?)
    }

    /// [`Store::transaction`] bound to a caller operation id. One attribution row
    /// maps to the batch's consecutive journal events; a batch that commits no
    /// event (all no-ops) writes no row.
    pub fn transaction_op(
        &mut self,
        machine: &dyn Machine,
        req: &TransactionRequest,
        operation_id: &OperationId,
    ) -> Result<OperationOutcome<Vec<CommandResult>>, Diagnostic> {
        self.transaction_inner(machine, req, Some(operation_id))
    }

    fn transaction_inner(
        &mut self,
        machine: &dyn Machine,
        req: &TransactionRequest,
        operation_id: Option<&OperationId>,
    ) -> Result<OperationOutcome<Vec<CommandResult>>, Diagnostic> {
        let binding = operation_id.map(|id| Binding::new(id, &OperationRequest::Transaction(req)));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if let Some(binding) = &binding {
            let first = load_snapshot(&tx, &req.aggregate_id)?;
            check_machine_match(&first, machine.identity(), &req.machine)?;
            if let Some(history) = resolve_duplicate(&tx, &req.aggregate_id, binding)? {
                return Ok(OperationOutcome::Historical(history));
            }
        }
        let mut committed: Vec<(runtime::JournalEvent, Receipt)> = Vec::new();
        let last = req.steps.len().saturating_sub(1);
        let mut capability = req.capability.clone();
        let mut results = Vec::with_capacity(req.steps.len());
        for index in 0..req.steps.len() {
            let stepped = run_transaction_step(
                &tx,
                machine,
                req,
                &capability,
                (index, last),
                #[cfg(feature = "test-support")]
                &mut self.crash_hook,
            );
            let (result, event) = match stepped {
                Ok(stepped) => stepped,
                Err(err) => return Err(fail_transaction(tx, err, index)),
            };
            capability = result.capability.clone();
            committed.extend(event);
            results.push(result);
        }
        let pairs: Vec<(&runtime::JournalEvent, &Receipt)> =
            committed.iter().map(|(e, r)| (e, r)).collect();
        bind_operation(
            &tx,
            &req.aggregate_id,
            binding.as_ref(),
            &pairs,
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
            #[cfg(feature = "test-support")]
            "transaction",
        )?;
        tx.commit().map_err(map_sqlite)?;
        Ok(OperationOutcome::Committed(results))
    }

    /// Bind an effect outcome to the exact pending identity (kernel `record`).
    pub fn record_effect(
        &mut self,
        req: &EffectOutcomeRequest,
    ) -> Result<CommandResult, Diagnostic> {
        expect_committed(self.record_effect_inner(req, None)?)
    }

    /// [`Store::record_effect`] bound to a caller operation id. The attribution is
    /// independent of, and in addition to, the existing exact-outcome
    /// deduplication by effect identity.
    pub fn record_effect_op(
        &mut self,
        req: &EffectOutcomeRequest,
        operation_id: &OperationId,
    ) -> Result<OperationOutcome<CommandResult>, Diagnostic> {
        self.record_effect_inner(req, Some(operation_id))
    }

    fn record_effect_inner(
        &mut self,
        req: &EffectOutcomeRequest,
        operation_id: Option<&OperationId>,
    ) -> Result<OperationOutcome<CommandResult>, Diagnostic> {
        let binding = operation_id.map(|id| Binding::new(id, &OperationRequest::Record(req)));
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let snapshot = load_snapshot(&tx, &req.aggregate_id)?;
        if req.machine != snapshot.machine {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                crate::diagnostic::NO_RULE,
                "presented machine identity does not match the aggregate's recorded identity",
            ));
        }
        if let Some(binding) = &binding {
            if let Some(history) = resolve_duplicate(&tx, &req.aggregate_id, binding)? {
                return Ok(OperationOutcome::Historical(history));
            }
        }
        let recorded_receipt = fetch_recorded_receipt(&tx, &snapshot)?;
        let step = runtime::execute_outcome_binding(
            &snapshot,
            recorded_receipt.as_ref(),
            &req.capability,
            &req.effect,
        )?;
        finish_step(
            tx,
            &snapshot,
            step,
            binding.as_ref(),
            #[cfg(feature = "test-support")]
            &mut self.crash_hook,
            #[cfg(feature = "test-support")]
            "record",
        )
        .map(OperationOutcome::Committed)
    }

    /// Read-only projection of an aggregate.
    pub fn show(&mut self, aggregate_id: &AggregateId) -> Result<Projection, Diagnostic> {
        let snapshot = load_snapshot(&self.conn, aggregate_id)?;
        let last_receipt = if snapshot.last_receipt_id.is_empty() {
            None
        } else {
            fetch_receipt(&self.conn, aggregate_id, &snapshot.last_receipt_id)?
        };
        Ok(Projection {
            aggregate_id: snapshot.aggregate_id.clone(),
            machine: snapshot.machine.clone(),
            state: snapshot.state.clone(),
            terminal: snapshot.terminal,
            generation: snapshot.generation,
            capability: Capability {
                generation: snapshot.generation,
                nonce: snapshot.nonce.clone(),
            },
            effect: snapshot.effect.clone(),
            sequence: snapshot.sequence,
            last_event_hash: snapshot.last_event_hash.clone(),
            last_receipt,
        })
    }

    /// Read-only: did this caller operation commit, as of this read? Reports an
    /// exact committed match (historical, never a current handle), a conflict
    /// (the id names a different request), or "not recorded as of this read". The
    /// last is not a refusal, a cancellation, or permission to retry under a new
    /// id: the original request may still be running or may never have arrived.
    pub fn lookup_operation(
        &mut self,
        operation_id: &OperationId,
        request: &OperationRequest<'_>,
    ) -> Result<Lookup, Diagnostic> {
        // One deferred read transaction so the aggregate coordinates and the
        // operation row describe the same committed snapshot.
        let tx = self.conn.unchecked_transaction().map_err(map_sqlite)?;
        let aggregate_id = request.aggregate_id();
        let exists = tx
            .query_row(
                "SELECT 1 FROM aggregates WHERE aggregate_id = ?1",
                params![aggregate_id.0],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_sqlite)?;
        if exists.is_none() {
            if matches!(request.kind(), OperationKind::CreateWithCommand) {
                return Ok(Lookup::NotRecorded {
                    current_generation: None,
                    current_sequence: None,
                });
            }
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("aggregate.not_found"),
                format!("aggregate `{}` not found", aggregate_id.0),
            ));
        }
        let snapshot = load_snapshot(&tx, aggregate_id)?;
        if request.machine() != &snapshot.machine {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                crate::diagnostic::NO_RULE,
                "presented machine identity does not match the aggregate's recorded identity",
            ));
        }
        let Some(record) = lookup_operation_row(&tx, aggregate_id, operation_id.as_str())? else {
            return Ok(Lookup::NotRecorded {
                current_generation: Some(snapshot.generation),
                current_sequence: Some(snapshot.sequence),
            });
        };
        if record.fingerprint != request.fingerprint() {
            return Ok(Lookup::Conflict {
                kind: record.kind,
                first_sequence: record.first_sequence,
                last_sequence: record.last_sequence,
                current_generation: snapshot.generation,
                current_sequence: snapshot.sequence,
            });
        }
        Ok(Lookup::CommittedExact(OperationHistory {
            record,
            current_generation: snapshot.generation,
            current_sequence: snapshot.sequence,
        }))
    }

    /// Rebuild the projection from the journal for one aggregate (or all). Refuses
    /// journal corruption; repairs a projection that disagrees with replay.
    pub fn replay(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: Option<&AggregateId>,
    ) -> Result<ReplaySummary, Diagnostic> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let ids = aggregate_ids(&tx, aggregate_id)?;
        let mut entries = Vec::new();
        for id in ids {
            let agg = AggregateId(id.clone());
            let events = load_events(&tx, &agg)?;
            verify_chain(&events)?;
            let snapshot = load_snapshot(&tx, &agg)?;
            if machine.identity() != &snapshot.machine {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkMachineMismatch,
                    crate::diagnostic::NO_RULE,
                    "the loaded machine does not match the aggregate's recorded identity",
                ));
            }
            let replayed = replay_events(machine, &events)?;
            let matches = replayed.state == snapshot.state
                && replayed.terminal == snapshot.terminal
                && replayed.generation == snapshot.generation
                && replayed.nonce == snapshot.nonce
                && replayed.effect == snapshot.effect
                && replayed.sequence == snapshot.sequence
                && replayed.last_event_hash == snapshot.last_event_hash
                && snapshot.last_receipt_id == replayed.last_event_hash;
            if matches {
                entries.push(ReplayEntry {
                    aggregate_id: agg,
                    sequence: replayed.sequence,
                    generation: replayed.generation,
                    state: replayed.state,
                    rebuilt: false,
                });
            } else {
                // Rebuild the projection and the derived receipts.
                let nonce = runtime::derive_nonce(&snapshot.machine, &agg, replayed.generation);
                tx.execute(
                    "DELETE FROM receipts WHERE aggregate_id = ?1",
                    params![agg.0],
                )
                .map_err(map_sqlite)?;
                for event in &events {
                    let receipt = runtime::receipt_from_event(event);
                    tx.execute(
                        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES (?1, ?2, ?3, ?4)",
                        params![agg.0, event.sequence as i64, receipt.receipt_id, canonical_json(&receipt)],
                    )
                    .map_err(map_sqlite)?;
                }
                tx.execute(
                    "UPDATE aggregates SET state = ?1, terminal = ?2, generation = ?3, nonce = ?4, effect_json = ?5, sequence = ?6, last_event_hash = ?7, last_receipt_id = ?8 WHERE aggregate_id = ?9",
                    params![
                        canonical_json(&replayed.state),
                        replayed.terminal as i64,
                        replayed.generation as i64,
                        nonce,
                        canonical_json(&replayed.effect),
                        replayed.sequence as i64,
                        replayed.last_event_hash,
                        replayed.last_event_hash,
                        agg.0,
                    ],
                )
                .map_err(map_sqlite)?;
                entries.push(ReplayEntry {
                    aggregate_id: agg,
                    sequence: replayed.sequence,
                    generation: replayed.generation,
                    state: replayed.state,
                    rebuilt: true,
                });
            }
        }
        tx.commit().map_err(map_sqlite)?;
        Ok(ReplaySummary {
            store_id: self.store_id.clone(),
            aggregates: entries,
        })
    }

    /// Verify journal integrity and projection agreement for one aggregate (or all).
    /// Refuses on journal corruption, replay mismatch, or projection corruption.
    pub fn verify(
        &mut self,
        machine: &dyn Machine,
        aggregate_id: Option<&AggregateId>,
    ) -> Result<VerifySummary, Diagnostic> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_sqlite)?;
        let ids = aggregate_ids(&tx, aggregate_id)?;
        let aggregate_count = ids.len();
        for id in ids {
            let agg = AggregateId(id);
            let events = load_events(&tx, &agg)?;
            verify_chain(&events)?;
            verify_receipts(&tx, &events)?;
            let snapshot = load_snapshot(&tx, &agg)?;
            if machine.identity() != &snapshot.machine {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkMachineMismatch,
                    crate::diagnostic::NO_RULE,
                    "the loaded machine does not match the aggregate's recorded identity",
                ));
            }
            let replayed = replay_events(machine, &events)?;
            let matches = replayed.state == snapshot.state
                && replayed.terminal == snapshot.terminal
                && replayed.generation == snapshot.generation
                && replayed.nonce == snapshot.nonce
                && replayed.effect == snapshot.effect
                && replayed.sequence == snapshot.sequence
                && replayed.last_event_hash == snapshot.last_event_hash
                && snapshot.last_receipt_id == replayed.last_event_hash;
            if !matches {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkProjectionCorrupt,
                    Some("projection.replay"),
                    "stored projection disagrees with journal replay",
                ));
            }
        }
        // Receipt rows keyed to an aggregate that does not exist are invisible to
        // the per-aggregate set equality above; reject them as orphan projection
        // rows so every stored receipt belongs to a real journal.
        verify_no_orphan_receipts(&tx)?;
        verify_operations(&tx, aggregate_id)?;
        drop(tx);
        Ok(VerifySummary {
            store_id: self.store_id.clone(),
            aggregates: aggregate_count,
            journal_ok: true,
            projection_ok: true,
            verified: true,
        })
    }
}

/// Summary returned by read-only whole-store verification (`ik store verify`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadonlyVerifyReport {
    pub store_id: String,
    pub aggregates: usize,
    pub events: usize,
    pub operations: usize,
    pub mode: &'static str,
}

/// Verify an entire SQLite store over a strictly read-only `rusqlite::Connection`.
///
/// Always checks schema metadata, zero orphan rows in `receipts` / `journal` /
/// `operations`, every operation binding, every aggregate's full hash chain and
/// deterministic receipts, and every materialized `aggregates` projection row
/// against the journal-derived state digest, terminality, generation, nonce,
/// effect status, sequence, and tip receipt/event hash.
///
/// When `machines` is `Some(list)`, additionally requires every aggregate's
/// `(machine_id, version, spec_hash)` to match one supplied machine and replays
/// every event through `replay_events`.
pub(crate) fn verify_connection_readonly(
    conn: &Connection,
    machines: Option<&[Box<dyn Machine>]>,
) -> Result<ReadonlyVerifyReport, Diagnostic> {
    let quick: String = conn
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    if quick != "ok" {
        return Err(Diagnostic::new(
            DiagnosticCode::IkStoreCorrupt,
            Some("store.quick_check"),
            format!("SQLite quick_check failed: {quick}"),
        ));
    }
    let schema_version = meta_value(conn, "schema_version")?;
    if schema_version != SCHEMA_VERSION.to_string() {
        return Err(Diagnostic::new(
            DiagnosticCode::IkStoreCorrupt,
            crate::diagnostic::NO_RULE,
            format!("unsupported store schema version `{schema_version}` (expected {SCHEMA_VERSION})"),
        ));
    }
    let store_id = meta_value(conn, "store_id")?;

    verify_no_orphan_receipts(conn)?;
    let orphan_journal: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal WHERE aggregate_id NOT IN (SELECT aggregate_id FROM aggregates)",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite)?;
    if orphan_journal != 0 {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProjectionCorrupt,
            Some("journal.orphan"),
            format!("found {orphan_journal} orphan journal row(s) without an aggregate"),
        ));
    }
    let orphan_ops: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE aggregate_id NOT IN (SELECT aggregate_id FROM aggregates)",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite)?;
    if orphan_ops != 0 {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProjectionCorrupt,
            Some("operations.orphan"),
            format!("found {orphan_ops} orphan operation row(s) without an aggregate"),
        ));
    }

    verify_operations(conn, None)?;
    let total_operations: i64 = conn
        .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    let ids = aggregate_ids(conn, None)?;
    let mut total_events: usize = 0;

    for id in &ids {
        let agg = AggregateId(id.clone());
        let events = load_events(conn, &agg)?;
        total_events += events.len();
        verify_chain(&events)?;
        verify_receipts(conn, &events)?;
        let snapshot = load_snapshot(conn, &agg)?;

        let mut effect = EffectStatus::None;
        for event in &events {
            if event.aggregate_id != agg
                || event.machine_id != snapshot.machine.machine_id
                || event.machine_version != snapshot.machine.version.0
                || event.machine_spec_hash != snapshot.machine.spec_hash.0
            {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkJournalCorrupt,
                    Some("journal.identity"),
                    format!(
                        "journal identity disagrees with aggregate `{}` at sequence {}",
                        agg.0, event.sequence
                    ),
                ));
            }
            effect = runtime::apply_effect_delta(&effect, &event.effect_delta, &event.hash);
        }

        let last = events.last().expect("verify_chain ensures non-empty journal");
        let expected_nonce =
            runtime::derive_nonce(&snapshot.machine, &agg, last.generation);
        let expected_receipt_id = runtime::receipt_from_event(last).receipt_id;
        let projection_matches = digest_json(&snapshot.state) == last.state_digest
            && snapshot.terminal == last.terminal
            && snapshot.generation == last.generation
            && snapshot.nonce == expected_nonce
            && snapshot.effect == effect
            && snapshot.sequence == last.sequence
            && snapshot.last_event_hash == last.hash
            && snapshot.last_receipt_id == expected_receipt_id;
        if !projection_matches {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProjectionCorrupt,
                Some("projection.replay"),
                format!("stored projection for `{}` disagrees with journal", agg.0),
            ));
        }

        if let Some(loaded) = machines {
            let machine = loaded
                .iter()
                .find(|m| m.identity() == &snapshot.machine)
                .ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticCode::IkMachineMismatch,
                        crate::diagnostic::NO_RULE,
                        format!(
                            "no `--machine` matched aggregate `{}` (`{}` v{} {})",
                            agg.0,
                            snapshot.machine.machine_id,
                            snapshot.machine.version.0,
                            snapshot.machine.spec_hash.0
                        ),
                    )
                })?;
            let replayed = replay_events(machine.as_ref(), &events)?;
            let semantic_matches = replayed.state == snapshot.state
                && replayed.terminal == snapshot.terminal
                && replayed.generation == snapshot.generation
                && replayed.nonce == snapshot.nonce
                && replayed.effect == snapshot.effect
                && replayed.sequence == snapshot.sequence
                && replayed.last_event_hash == snapshot.last_event_hash;
            if !semantic_matches {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkProjectionCorrupt,
                    Some("projection.replay"),
                    format!(
                        "stored projection for `{}` disagrees with machine replay",
                        agg.0
                    ),
                ));
            }
        }
    }

    Ok(ReadonlyVerifyReport {
        store_id,
        aggregates: ids.len(),
        events: total_events,
        operations: usize::try_from(total_operations).unwrap_or(0),
        mode: if machines.is_some() {
            "semantic"
        } else {
            "cryptographic"
        },
    })
}

/// Pure, deterministic construction of the genesis event, its receipt, and the
/// execution step for the initial command of an atomic create-with-command
/// request. Deterministic in (machine, aggregate id, command, observation), so the
/// exact-retry path can compare the stored journal against exactly what this
/// request would have written. The genesis capability (generation 0) is always
/// current for a fresh aggregate, so no caller capability is consulted.
fn build_create_command_plan(
    machine: &dyn Machine,
    aggregate_id: &AggregateId,
    command: &Value,
    observation: Option<&Value>,
) -> Result<(runtime::GenesisPlan, Step), Diagnostic> {
    let identity = machine.identity().clone();
    let initial = machine.initial_state();
    let create_command = crate::declarative::compile::create_command(&aggregate_id.0, &identity);
    let genesis = runtime::genesis_plan(&identity, aggregate_id, initial, create_command);
    let nonce0 = runtime::derive_nonce(&identity, aggregate_id, 0);
    let snapshot = AggregateSnapshot {
        aggregate_id: aggregate_id.clone(),
        machine: identity,
        state: genesis.state.clone(),
        terminal: false,
        generation: 0,
        nonce: nonce0.clone(),
        effect: EffectStatus::None,
        sequence: 0,
        last_event_hash: genesis.event.hash.clone(),
        last_receipt_id: genesis.receipt.receipt_id.clone(),
    };
    let capability = Capability {
        generation: 0,
        nonce: nonce0,
    };
    let step =
        runtime::execute_command(machine, &snapshot, None, &capability, command, observation)?;
    Ok((genesis, step))
}

/// Refuse an invalid genesis. A test-support mutant deletes this wall.
fn validate_genesis(machine: &dyn Machine) -> Result<(), Diagnostic> {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::SkipGenesisValidation) {
        return Ok(());
    }
    machine.validate_initial()
}

fn check_create_identity(
    loaded: &MachineIdentity,
    presented: &MachineIdentity,
) -> Result<(), Diagnostic> {
    if presented.machine_id != loaded.machine_id {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineUnknown,
            crate::diagnostic::NO_RULE,
            format!("unknown machine: {}", presented.machine_id),
        ));
    }
    if presented.version != loaded.version {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineVersionUnknown,
            crate::diagnostic::NO_RULE,
            format!(
                "unknown machine version {} for {}",
                presented.version.0, presented.machine_id
            ),
        ));
    }
    if presented.spec_hash != loaded.spec_hash {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineHashMismatch,
            crate::diagnostic::NO_RULE,
            format!("machine spec hash mismatch for {}", presented.machine_id),
        ));
    }
    Ok(())
}

fn check_machine_match(
    snapshot: &AggregateSnapshot,
    loaded: &MachineIdentity,
    presented: &MachineIdentity,
) -> Result<(), Diagnostic> {
    if presented != &snapshot.machine {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineMismatch,
            crate::diagnostic::NO_RULE,
            "presented machine identity does not match the aggregate's recorded identity",
        ));
    }
    if loaded != &snapshot.machine {
        return Err(Diagnostic::new(
            DiagnosticCode::IkMachineMismatch,
            crate::diagnostic::NO_RULE,
            "the loaded machine does not match the aggregate's recorded identity",
        ));
    }
    Ok(())
}

fn load_snapshot(
    conn: &Connection,
    aggregate_id: &AggregateId,
) -> Result<AggregateSnapshot, Diagnostic> {
    let row = conn
        .query_row(
            "SELECT machine_id, machine_version, machine_spec_hash, state, terminal, generation, nonce, effect_json, sequence, last_event_hash, last_receipt_id FROM aggregates WHERE aggregate_id = ?1",
            params![aggregate_id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite)?;
    let Some((
        machine_id,
        machine_version,
        machine_spec_hash,
        state_json,
        terminal,
        generation,
        nonce,
        effect_json,
        sequence,
        last_event_hash,
        last_receipt_id,
    )) = row
    else {
        return Err(Diagnostic::new(
            DiagnosticCode::IkIllegalState,
            Some("aggregate.not_found"),
            format!("aggregate `{}` not found", aggregate_id.0),
        ));
    };
    let machine = MachineIdentity {
        machine_id,
        version: MachineVersion(checked_u64(
            DiagnosticCode::IkStoreCorrupt,
            "store.negative_machine_version",
            "machine_version",
            machine_version,
        )?),
        spec_hash: MachineSpecHash(machine_spec_hash),
    };
    let state: Value = serde_json::from_str(&state_json)
        .map_err(|e| Diagnostic::internal(format!("stored state decode failed: {e}")))?;
    let effect: EffectStatus = serde_json::from_str(&effect_json)
        .map_err(|e| Diagnostic::internal(format!("stored effect status decode failed: {e}")))?;
    // `terminal` is a persisted boolean; anything but exactly 0/1 is storage
    // corruption. A nonzero-anything truthiness would silently promote a forged
    // value to a terminal aggregate.
    let terminal = match terminal {
        0 => false,
        1 => true,
        _ => {
            return Err(Diagnostic::new(
                DiagnosticCode::IkStoreCorrupt,
                Some("store.terminal"),
                format!("stored terminal is not boolean (0/1): {terminal}"),
            ))
        }
    };
    Ok(AggregateSnapshot {
        aggregate_id: aggregate_id.clone(),
        machine,
        state,
        terminal,
        generation: checked_u64(
            DiagnosticCode::IkStoreCorrupt,
            "store.negative_generation",
            "generation",
            generation,
        )?,
        nonce,
        effect,
        sequence: checked_u64(
            DiagnosticCode::IkStoreCorrupt,
            "store.negative_sequence",
            "sequence",
            sequence,
        )?,
        last_event_hash,
        last_receipt_id,
    })
}

/// Checked signed-to-unsigned decode of a persisted authority coordinate. SQLite
/// `INTEGER` is `i64`, and the old `as u64` cast silently wrapped a negative
/// persisted value into a huge positive one. Every persisted coordinate must be
/// non-negative; a negative value is storage corruption and refuses with the given
/// stable `code`/`rule` before any command, replay, or repair can proceed.
fn checked_u64(
    code: DiagnosticCode,
    rule: &str,
    what: &str,
    value: i64,
) -> Result<u64, Diagnostic> {
    u64::try_from(value).map_err(|_| {
        Diagnostic::new(
            code,
            Some(rule),
            format!("stored {what} is negative ({value}); expected a non-negative integer"),
        )
    })
}

fn fetch_receipt(
    conn: &Connection,
    aggregate_id: &AggregateId,
    receipt_id: &str,
) -> Result<Option<Receipt>, Diagnostic> {
    let json: Option<String> = conn
        .query_row(
            "SELECT receipt_json FROM receipts WHERE aggregate_id = ?1 AND receipt_id = ?2",
            params![aggregate_id.0, receipt_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    match json {
        Some(json) => serde_json::from_str(&json)
            .map(Some)
            .map_err(|e| Diagnostic::internal(format!("stored receipt decode failed: {e}"))),
        None => Ok(None),
    }
}

fn fetch_recorded_receipt(
    conn: &Connection,
    snapshot: &AggregateSnapshot,
) -> Result<Option<Receipt>, Diagnostic> {
    match &snapshot.effect {
        EffectStatus::Recorded(_, _, Some(receipt_id)) => {
            fetch_receipt(conn, &snapshot.aggregate_id, receipt_id)
        }
        _ => Ok(None),
    }
}

/// Invoke the injected test-only crash hook at a named persistence boundary.
/// Returning `false` keeps it armed for a later boundary in the same operation;
/// returning `true` consumes it so it can fire at most once per store.
#[cfg(feature = "test-support")]
fn fire_crash(crash_hook: &mut Option<CrashHook>, point: &str) {
    let consumed = crash_hook.as_mut().is_some_and(|hook| hook(point));
    if consumed {
        crash_hook.take();
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the two extra parameters exist only in the non-default crash-test feature"
)]
fn persist_plan(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    event: &JournalEvent,
    receipt: &Receipt,
    effect_after: &EffectStatus,
    canonical_state: &str,
    nonce: &str,
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
    #[cfg(feature = "test-support")] kind: &str,
) -> Result<(), Diagnostic> {
    // SQLite `INTEGER` is i64; a sequence/generation past `i64::MAX` would silently
    // wrap negative on an `as i64` cast and be re-read as a forged authority
    // coordinate. The runtime's u64-ceiling check alone is not enough (2^63 fits
    // in u64 but not in i64), so refuse before any journal/receipt/projection
    // write with the same counter-overflow family.
    let sequence = checked_persist_i64("sequence", event.sequence)?;
    let generation = checked_persist_i64("generation", event.generation)?;
    tx.execute(
        "INSERT INTO journal (aggregate_id, sequence, generation, hash, prev_hash, event_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            aggregate_id.0,
            sequence,
            generation,
            event.hash,
            event.prev_hash,
            canonical_json(event)
        ],
    )
    .map_err(map_sqlite)?;
    #[cfg(feature = "test-support")]
    fire_crash(crash_hook, &format!("crash.{kind}.after_journal"));
    tx.execute(
        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES (?1, ?2, ?3, ?4)",
        params![
            aggregate_id.0,
            sequence,
            receipt.receipt_id,
            canonical_json(receipt)
        ],
    )
    .map_err(map_sqlite)?;
    #[cfg(feature = "test-support")]
    fire_crash(crash_hook, &format!("crash.{kind}.after_receipts"));
    tx.execute(
        "UPDATE aggregates SET state = ?1, terminal = ?2, generation = ?3, nonce = ?4, effect_json = ?5, sequence = ?6, last_event_hash = ?7, last_receipt_id = ?8 WHERE aggregate_id = ?9",
        params![
            canonical_state,
            event.terminal as i64,
            generation,
            nonce,
            canonical_json(effect_after),
            sequence,
            event.hash,
            receipt.receipt_id,
            aggregate_id.0
        ],
    )
    .map_err(map_sqlite)?;
    #[cfg(feature = "test-support")]
    fire_crash(crash_hook, &format!("crash.{kind}.after_projection"));
    Ok(())
}

/// Checked u64-to-i64 conversion for a persisted coordinate. SQLite `INTEGER` is
/// `i64`; a value past `i64::MAX` would silently wrap negative on an `as i64`
/// cast. Refuse with the counter-overflow diagnostic before any write so overflow
/// can never mutate journal/receipt/projection rows.
fn checked_persist_i64(counter: &str, value: u64) -> Result<i64, Diagnostic> {
    i64::try_from(value).map_err(|_| {
        Diagnostic::new(
            DiagnosticCode::IkCounterOverflow,
            Some(counter),
            format!("{counter} {value} exceeds the SQLite INTEGER range (i64::MAX)"),
        )
    })
}

fn expect_committed<T>(outcome: OperationOutcome<T>) -> Result<T, Diagnostic> {
    match outcome {
        OperationOutcome::Committed(value) => Ok(value),
        OperationOutcome::Historical(_) => Err(Diagnostic::internal(
            "historical result without an operation id",
        )),
    }
}

fn operation_row_corrupt(operation_id: &str, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IkStoreCorrupt,
        Some("operations.row"),
        format!(
            "operation `{operation_id}` row is unreadable: {}",
            detail.into()
        ),
    )
}

/// Read one aggregate-scoped operation row.
fn lookup_operation_row(
    conn: &Connection,
    aggregate_id: &AggregateId,
    operation_id: &str,
) -> Result<Option<OperationRecord>, Diagnostic> {
    let raw = conn
        .query_row(
            "SELECT kind, fingerprint, first_sequence, last_sequence, generation, state_digest, event_hashes, receipt_ids FROM operations WHERE aggregate_id = ?1 AND operation_id = ?2",
            params![aggregate_id.0, operation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite)?;
    let Some((kind, fingerprint, first, last, generation, state_digest, hashes, receipts)) = raw
    else {
        return Ok(None);
    };
    let kind = OperationKind::parse(&kind)
        .ok_or_else(|| operation_row_corrupt(operation_id, format!("unknown kind `{kind}`")))?;
    let as_u64 = |value: i64, field: &str| {
        u64::try_from(value)
            .map_err(|_| operation_row_corrupt(operation_id, format!("negative {field}")))
    };
    let list = |text: &str, field: &str| {
        serde_json::from_str::<Vec<String>>(text)
            .map_err(|e| operation_row_corrupt(operation_id, format!("{field}: {e}")))
    };
    Ok(Some(OperationRecord {
        operation_id: operation_id.to_string(),
        kind,
        fingerprint,
        first_sequence: as_u64(first, "first_sequence")?,
        last_sequence: as_u64(last, "last_sequence")?,
        generation: as_u64(generation, "generation")?,
        state_digest,
        event_hashes: list(&hashes, "event_hashes")?,
        receipt_ids: list(&receipts, "receipt_ids")?,
    }))
}

/// Write one operation row inside the mutation's own transaction.
fn insert_operation(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    record: &OperationRecord,
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
    #[cfg(feature = "test-support")] kind: &str,
) -> Result<(), Diagnostic> {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::SkipOperationBinding) {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO operations (aggregate_id, operation_id, kind, fingerprint, first_sequence, last_sequence, generation, state_digest, event_hashes, receipt_ids) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            aggregate_id.0,
            record.operation_id,
            record.kind.as_str(),
            record.fingerprint,
            checked_persist_i64("sequence", record.first_sequence)?,
            checked_persist_i64("sequence", record.last_sequence)?,
            checked_persist_i64("generation", record.generation)?,
            record.state_digest,
            canonical_json(&record.event_hashes),
            canonical_json(&record.receipt_ids),
        ],
    )
    .map_err(map_sqlite)?;
    #[cfg(feature = "test-support")]
    fire_crash(crash_hook, &format!("crash.{kind}.after_operation"));
    Ok(())
}

fn history_for(
    conn: &Connection,
    aggregate_id: &AggregateId,
    record: OperationRecord,
) -> Result<OperationHistory, Diagnostic> {
    let snapshot = load_snapshot(conn, aggregate_id)?;
    Ok(OperationHistory {
        record,
        current_generation: snapshot.generation,
        current_sequence: snapshot.sequence,
    })
}

fn operation_conflict(record: &OperationRecord) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IkOperationConflict,
        Some("operation.fingerprint"),
        format!(
            "operation `{}` already committed a different {} request",
            record.operation_id,
            record.kind.as_str()
        ),
    )
}

/// Duplicate gate, run before currentness: an exact repeat returns the recorded
/// history, a different request under the same id refuses, an unknown id continues.
fn resolve_duplicate(
    conn: &Connection,
    aggregate_id: &AggregateId,
    binding: &Binding,
) -> Result<Option<OperationHistory>, Diagnostic> {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::SkipDuplicateGate) {
        return Ok(None);
    }
    let Some(record) = lookup_operation_row(conn, aggregate_id, binding.operation_id.as_str())?
    else {
        return Ok(None);
    };
    if record.fingerprint != binding.fingerprint {
        return Err(operation_conflict(&record));
    }
    history_for(conn, aggregate_id, record).map(Some)
}

/// Pre-transaction duplicate probe for create-with-command, so an exact repeat
/// never reaches the reducer. When the aggregate already exists its machine
/// identity is checked first, so a wrong-machine caller learns nothing about the
/// recorded operation.
fn recorded_history(
    conn: &Connection,
    machine: &dyn Machine,
    presented: &MachineIdentity,
    aggregate_id: &AggregateId,
    binding: &Binding,
) -> Result<Option<OperationHistory>, Diagnostic> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM aggregates WHERE aggregate_id = ?1",
            params![aggregate_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_sqlite)?;
    if exists.is_none() {
        return Ok(None);
    }
    let snapshot = load_snapshot(conn, aggregate_id)?;
    check_machine_match(&snapshot, machine.identity(), presented)?;
    resolve_duplicate(conn, aggregate_id, binding)
}

/// Exact repeat of a committed create-with-command operation, answered from its
/// record before any reducer or worker runs.
fn probe_recorded_history(
    (conn, machine, presented, aggregate_id): (
        &Connection,
        &dyn Machine,
        &MachineIdentity,
        &AggregateId,
    ),
    binding: Option<&Binding>,
) -> Result<Option<OperationHistory>, Diagnostic> {
    let Some(binding) = binding else {
        return Ok(None);
    };
    recorded_history(conn, machine, presented, aggregate_id, binding)
}

/// Inside the write transaction: a recorded operation or an exact existing
/// admission answers without writing.
fn existing_admission(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    binding: Option<&Binding>,
    genesis: &runtime::JournalEvent,
    step: &Step,
) -> Result<Option<OperationOutcome<CreateWithCommandOutcome>>, Diagnostic> {
    if let Some(binding) = binding {
        if let Some(history) = resolve_duplicate(tx, aggregate_id, binding)? {
            return Ok(Some(OperationOutcome::Historical(history)));
        }
    }
    let exact = exact_duplicate_result(tx, aggregate_id, genesis, step)?;
    Ok(exact.map(|result| {
        OperationOutcome::Committed(CreateWithCommandOutcome::ExactDuplicate(result))
    }))
}

/// A failed transaction step abandons the whole transaction (drop = rollback).
/// The `CommitPartialTransaction` mutant deliberately commits the prefix instead.
#[cfg_attr(not(feature = "test-support"), allow(unused_variables))]
fn fail_transaction(tx: Transaction, err: Diagnostic, index: usize) -> Diagnostic {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::CommitPartialTransaction) && index > 0 {
        let _ = tx.commit();
    }
    err
}

/// Execute one step of a transaction inside `tx`. Returns the step's result and,
/// when it committed an event, that event with its receipt.
fn run_transaction_step(
    tx: &Transaction,
    machine: &dyn Machine,
    req: &TransactionRequest,
    capability: &Capability,
    (index, last): (usize, usize),
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
) -> Result<(CommandResult, Option<(runtime::JournalEvent, Receipt)>), Diagnostic> {
    let step = &req.steps[index];
    let snapshot = load_snapshot(tx, &req.aggregate_id)?;
    check_machine_match(&snapshot, machine.identity(), &req.machine)?;
    let recorded_receipt = fetch_recorded_receipt(tx, &snapshot)?;
    let planned = runtime::execute_command(
        machine,
        &snapshot,
        recorded_receipt.as_ref(),
        capability,
        &step.command,
        step.observation.as_ref(),
    )
    .map_err(|err| in_step(err, index))?;
    match planned {
        Step::Noop { note } => Ok((unchanged_result(&snapshot, capability, None, note), None)),
        Step::DuplicateReplay(receipt) => Ok((
            unchanged_result(&snapshot, capability, Some(*receipt), None),
            None,
        )),
        Step::Commit(plan) => {
            check_effect_boundary(&plan.effect_after, &snapshot.effect, index, last)?;
            let nonce = runtime::derive_nonce(
                &snapshot.machine,
                &snapshot.aggregate_id,
                plan.event.generation,
            );
            persist_plan(
                tx,
                &snapshot.aggregate_id,
                &plan.event,
                &plan.receipt,
                &plan.effect_after,
                &plan.canonical_state,
                &nonce,
                #[cfg(feature = "test-support")]
                crash_hook,
                #[cfg(feature = "test-support")]
                "transaction",
            )?;
            let committed = (plan.event.clone(), plan.receipt.clone());
            let result = CommandResult {
                receipt: Some(plan.receipt),
                state: plan.state,
                terminal: plan.event.terminal,
                generation: plan.event.generation,
                capability: Capability {
                    generation: plan.event.generation,
                    nonce,
                },
                effect: plan.effect_after,
                mutation: true,
                note: plan.event.note,
            };
            Ok((result, Some(committed)))
        }
    }
}

/// Persist the admitted command (or none, for a no-op) after the genesis rows,
/// bind the operation, and commit.
#[allow(
    clippy::too_many_arguments,
    reason = "the extra parameter exists only in the non-default crash-test feature"
)]
fn commit_admission(
    tx: Transaction,
    aggregate_id: &AggregateId,
    identity: &MachineIdentity,
    binding: Option<&Binding>,
    (genesis_plan, nonce0): (runtime::GenesisPlan, String),
    step: Step,
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
) -> Result<CreateWithCommandOutcome, Diagnostic> {
    let genesis = &genesis_plan.event;
    let genesis_receipt = &genesis_plan.receipt;
    let result = match step {
        Step::Commit(plan) => {
            let nonce = runtime::derive_nonce(identity, aggregate_id, plan.event.generation);
            persist_plan(
                &tx,
                aggregate_id,
                &plan.event,
                &plan.receipt,
                &plan.effect_after,
                &plan.canonical_state,
                &nonce,
                #[cfg(feature = "test-support")]
                crash_hook,
                #[cfg(feature = "test-support")]
                "admit",
            )?;
            bind_operation(
                &tx,
                aggregate_id,
                binding,
                &[(genesis, genesis_receipt), (&plan.event, &plan.receipt)],
                #[cfg(feature = "test-support")]
                crash_hook,
                #[cfg(feature = "test-support")]
                "admit",
            )?;
            CommandResult {
                receipt: Some(plan.receipt),
                state: plan.state,
                terminal: plan.event.terminal,
                generation: plan.event.generation,
                capability: Capability {
                    generation: plan.event.generation,
                    nonce,
                },
                effect: plan.effect_after,
                mutation: true,
                note: plan.event.note,
            }
        }
        Step::Noop { note } => {
            bind_operation(
                &tx,
                aggregate_id,
                binding,
                &[(genesis, genesis_receipt)],
                #[cfg(feature = "test-support")]
                crash_hook,
                #[cfg(feature = "test-support")]
                "admit",
            )?;
            CommandResult {
                receipt: Some(genesis_plan.receipt.clone()),
                state: genesis_plan.state.clone(),
                terminal: false,
                generation: 0,
                capability: Capability {
                    generation: 0,
                    nonce: nonce0,
                },
                effect: EffectStatus::None,
                mutation: true,
                note,
            }
        }
        Step::DuplicateReplay(_) => {
            return Err(Diagnostic::internal(
                "duplicate outcome replay is impossible on a fresh create-with-command aggregate",
            ));
        }
    };
    tx.commit().map_err(map_sqlite)?;
    Ok(CreateWithCommandOutcome::Committed(result))
}

/// Write the operation row for the events one mutation committed (if it carries
/// an operation id and committed anything).
fn bind_operation(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    binding: Option<&Binding>,
    events: &[(&runtime::JournalEvent, &Receipt)],
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
    #[cfg(feature = "test-support")] kind: &str,
) -> Result<(), Diagnostic> {
    let Some(binding) = binding else {
        return Ok(());
    };
    if events.is_empty() {
        return Ok(());
    }
    let record = binding.record(events)?;
    insert_operation(
        tx,
        aggregate_id,
        &record,
        #[cfg(feature = "test-support")]
        crash_hook,
        #[cfg(feature = "test-support")]
        kind,
    )
}

/// Result of a step that changed nothing: the aggregate's current coordinates.
fn unchanged_result(
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    receipt: Option<Receipt>,
    note: Option<Value>,
) -> CommandResult {
    CommandResult {
        receipt,
        state: snapshot.state.clone(),
        terminal: snapshot.terminal,
        generation: snapshot.generation,
        capability: capability.clone(),
        effect: snapshot.effect.clone(),
        mutation: false,
        note,
    }
}

/// An effect intent may only be created by the last step of a transaction.
fn check_effect_boundary(
    effect_after: &EffectStatus,
    effect_before: &EffectStatus,
    index: usize,
    last: usize,
) -> Result<(), Diagnostic> {
    if index < last
        && matches!(effect_after, EffectStatus::Pending(_))
        && effect_after != effect_before
    {
        return Err(in_step(
            Diagnostic::new(
                DiagnosticCode::IkTransactionBoundary,
                Some("transaction.effect_boundary"),
                "an effect intent may only be created by the last step of a transaction",
            ),
            index,
        ));
    }
    Ok(())
}

/// An existing aggregate either equals EXACTLY what a create-with-command request
/// would have written (typed idempotent duplicate) or refuses; a divergent
/// admission is never accepted. `None` means the aggregate does not exist.
fn exact_duplicate_result(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    genesis: &runtime::JournalEvent,
    step: &Step,
) -> Result<Option<CommandResult>, Diagnostic> {
    let exists = tx
        .query_row(
            "SELECT 1 FROM aggregates WHERE aggregate_id = ?1",
            params![aggregate_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_sqlite)?;
    if exists.is_none() {
        return Ok(None);
    }
    let mut would_be = vec![genesis.clone()];
    if let Step::Commit(plan) = step {
        would_be.push(plan.event.clone());
    }
    if load_events(tx, aggregate_id)? != would_be {
        return Err(Diagnostic::new(
            DiagnosticCode::IkIllegalState,
            Some("create.command_conflict"),
            format!(
                "aggregate `{}` already exists but its genesis/admission does not \
                 exactly match this create-with-command request; divergent \
                 admission refused",
                aggregate_id.0,
            ),
        ));
    }
    let snapshot = load_snapshot(tx, aggregate_id)?;
    let last_receipt = if snapshot.last_receipt_id.is_empty() {
        None
    } else {
        fetch_receipt(tx, aggregate_id, &snapshot.last_receipt_id)?
    };
    let note = last_receipt
        .as_ref()
        .and_then(|receipt| receipt.note.clone());
    Ok(Some(CommandResult {
        receipt: last_receipt,
        state: snapshot.state.clone(),
        terminal: snapshot.terminal,
        generation: snapshot.generation,
        capability: Capability {
            generation: snapshot.generation,
            nonce: snapshot.nonce.clone(),
        },
        effect: snapshot.effect.clone(),
        mutation: false,
        note,
    }))
}

/// Persist the genesis event and the aggregate projection row.
fn insert_genesis(
    tx: &Transaction,
    aggregate_id: &AggregateId,
    identity: &MachineIdentity,
    (genesis, genesis_receipt): (&runtime::JournalEvent, &Receipt),
    canonical_state: &str,
    nonce0: &str,
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
) -> Result<(), Diagnostic> {
    persist_plan(
        tx,
        aggregate_id,
        genesis,
        genesis_receipt,
        &EffectStatus::None,
        canonical_state,
        nonce0,
        #[cfg(feature = "test-support")]
        crash_hook,
        #[cfg(feature = "test-support")]
        "genesis",
    )?;
    tx.execute(
        "INSERT INTO aggregates (aggregate_id, machine_id, machine_version, machine_spec_hash, state, terminal, generation, nonce, effect_json, sequence, last_event_hash, last_receipt_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            aggregate_id.0,
            identity.machine_id,
            identity.version.0 as i64,
            identity.spec_hash.0,
            canonical_state,
            0i64,
            0i64,
            nonce0,
            canonical_json(&EffectStatus::None),
            0i64,
            genesis.hash,
            genesis_receipt.receipt_id,
        ],
    )
    .map_err(map_sqlite)?;
    #[cfg(feature = "test-support")]
    fire_crash(crash_hook, "crash.genesis.after_aggregates_insert");
    Ok(())
}

/// Every operation row must name journal events that exist with the recorded hashes.
fn verify_operations(conn: &Connection, only: Option<&AggregateId>) -> Result<(), Diagnostic> {
    let keys: Vec<(String, String)> = if let Some(agg) = only {
        let mut stmt = conn
            .prepare_cached(
                "SELECT aggregate_id, operation_id FROM operations WHERE aggregate_id = ?1 ORDER BY operation_id",
            )
            .map_err(map_sqlite)?;
        let rows = stmt
            .query_map(params![agg.0], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(map_sqlite)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite)?;
        rows
    } else {
        let mut stmt = conn
            .prepare_cached(
                "SELECT aggregate_id, operation_id FROM operations ORDER BY aggregate_id, operation_id",
            )
            .map_err(map_sqlite)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(map_sqlite)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite)?;
        rows
    };
    for (aggregate, operation) in keys {
        verify_operation_binding(conn, &AggregateId(aggregate), &operation)?;
    }
    Ok(())
}

fn verify_operation_binding(
    conn: &Connection,
    aggregate_id: &AggregateId,
    operation: &str,
) -> Result<(), Diagnostic> {
    let record = lookup_operation_row(conn, aggregate_id, operation)?
        .ok_or_else(|| operation_row_corrupt(operation, "row vanished during verify"))?;
    let span = record
        .last_sequence
        .checked_sub(record.first_sequence)
        .map(|n| n + 1);
    if span != Some(record.event_hashes.len() as u64)
        || record.receipt_ids.len() != record.event_hashes.len()
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkJournalCorrupt,
            Some("operations.binding"),
            format!("operation `{operation}` sequence range disagrees with its recorded events"),
        ));
    }
    for (offset, hash) in record.event_hashes.iter().enumerate() {
        let sequence = checked_persist_i64("sequence", record.first_sequence + offset as u64)?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT hash FROM journal WHERE aggregate_id = ?1 AND sequence = ?2",
                params![aggregate_id.0, sequence],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sqlite)?;
        if stored.as_deref() != Some(hash.as_str()) {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("operations.binding"),
                format!("operation `{operation}` names an event that is not in the journal"),
            ));
        }
    }
    Ok(())
}

fn finish_step(
    tx: Transaction,
    snapshot: &AggregateSnapshot,
    step: Step,
    binding: Option<&Binding>,
    #[cfg(feature = "test-support")] crash_hook: &mut Option<CrashHook>,
    #[cfg(feature = "test-support")] kind: &str,
) -> Result<CommandResult, Diagnostic> {
    let current_capability = || Capability {
        generation: snapshot.generation,
        nonce: snapshot.nonce.clone(),
    };
    match step {
        Step::Noop { note } => {
            tx.commit().map_err(map_sqlite)?;
            Ok(CommandResult {
                receipt: None,
                state: snapshot.state.clone(),
                terminal: snapshot.terminal,
                generation: snapshot.generation,
                capability: current_capability(),
                effect: snapshot.effect.clone(),
                mutation: false,
                note,
            })
        }
        Step::DuplicateReplay(receipt) => {
            tx.commit().map_err(map_sqlite)?;
            Ok(CommandResult {
                receipt: Some(*receipt),
                state: snapshot.state.clone(),
                terminal: snapshot.terminal,
                generation: snapshot.generation,
                capability: current_capability(),
                effect: snapshot.effect.clone(),
                mutation: false,
                note: None,
            })
        }
        Step::Commit(plan) => {
            #[cfg(feature = "test-support")]
            let nonce = if crate::law_mutant::is(crate::law_mutant::LawMutant::SkipNonceRotation) {
                snapshot.nonce.clone()
            } else {
                runtime::derive_nonce(
                    &snapshot.machine,
                    &snapshot.aggregate_id,
                    plan.event.generation,
                )
            };
            #[cfg(not(feature = "test-support"))]
            let nonce = runtime::derive_nonce(
                &snapshot.machine,
                &snapshot.aggregate_id,
                plan.event.generation,
            );
            persist_plan(
                &tx,
                &snapshot.aggregate_id,
                &plan.event,
                &plan.receipt,
                &plan.effect_after,
                &plan.canonical_state,
                &nonce,
                #[cfg(feature = "test-support")]
                crash_hook,
                #[cfg(feature = "test-support")]
                kind,
            )?;
            if let Some(binding) = binding {
                let record = binding.record(&[(&plan.event, &plan.receipt)])?;
                insert_operation(
                    &tx,
                    &snapshot.aggregate_id,
                    &record,
                    #[cfg(feature = "test-support")]
                    crash_hook,
                    #[cfg(feature = "test-support")]
                    kind,
                )?;
            }
            tx.commit().map_err(map_sqlite)?;
            Ok(CommandResult {
                receipt: Some(plan.receipt),
                state: plan.state,
                terminal: plan.event.terminal,
                generation: plan.event.generation,
                capability: Capability {
                    generation: plan.event.generation,
                    nonce,
                },
                effect: plan.effect_after,
                mutation: true,
                note: plan.event.note,
            })
        }
    }
}

fn aggregate_ids(conn: &Connection, only: Option<&AggregateId>) -> Result<Vec<String>, Diagnostic> {
    match only {
        Some(id) => {
            let exists = conn
                .query_row(
                    "SELECT 1 FROM aggregates WHERE aggregate_id = ?1",
                    params![id.0],
                    |_| Ok(()),
                )
                .optional()
                .map_err(map_sqlite)?;
            if exists.is_none() {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkIllegalState,
                    Some("aggregate.not_found"),
                    format!("aggregate `{}` not found", id.0),
                ));
            }
            Ok(vec![id.0.clone()])
        }
        None => {
            let mut stmt = conn
                .prepare("SELECT aggregate_id FROM aggregates ORDER BY aggregate_id")
                .map_err(map_sqlite)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(map_sqlite)?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(map_sqlite)?);
            }
            Ok(ids)
        }
    }
}

fn load_events(
    conn: &Connection,
    aggregate_id: &AggregateId,
) -> Result<Vec<JournalEvent>, Diagnostic> {
    let mut stmt = conn
        .prepare(
            "SELECT sequence, generation, hash, prev_hash, event_json FROM journal WHERE aggregate_id = ?1 ORDER BY sequence",
        )
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map(params![aggregate_id.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(map_sqlite)?;
    let mut events = Vec::new();
    for row in rows {
        let (seq, gen, hash, prev_hash, json) = row.map_err(map_sqlite)?;
        // Reject negative persisted coordinates before the signed-to-unsigned
        // conversion; `as u64` would silently wrap them into huge positives.
        let sequence = checked_u64(
            DiagnosticCode::IkJournalCorrupt,
            "journal.negative_sequence",
            "journal sequence",
            seq,
        )?;
        let generation = checked_u64(
            DiagnosticCode::IkJournalCorrupt,
            "journal.negative_generation",
            "journal generation",
            gen,
        )?;
        let event: JournalEvent = serde_json::from_str(&json).map_err(|e| {
            Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                crate::diagnostic::NO_RULE,
                format!("journal event decode failed: {e}"),
            )
        })?;
        // The stored columns and the event payload must agree; a disagreement is
        // journal corruption, even if the payload itself is internally consistent.
        if event.sequence != sequence
            || event.generation != generation
            || event.hash != hash
            || event.prev_hash != prev_hash
        {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("journal.columns"),
                format!("journal row columns disagree with the event payload at sequence {seq}"),
            ));
        }
        events.push(event);
    }
    Ok(events)
}

/// Verify sequence contiguity, generation progression, and the hash chain.
fn verify_chain(events: &[JournalEvent]) -> Result<(), Diagnostic> {
    if events.is_empty() {
        return Err(Diagnostic::new(
            DiagnosticCode::IkJournalCorrupt,
            Some("journal.empty"),
            "journal is empty",
        ));
    }
    for (i, event) in events.iter().enumerate() {
        if event.sequence as usize != i {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("journal.sequence"),
                format!("journal sequence gap or reorder at index {i}"),
            ));
        }
        if event.generation != event.sequence {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("journal.generation"),
                format!(
                    "journal generation {} does not equal sequence {}",
                    event.generation, event.sequence
                ),
            ));
        }
        let expected_prev = if i == 0 {
            GENESIS_PREV
        } else {
            &events[i - 1].hash
        };
        if event.prev_hash != expected_prev {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("journal.chain"),
                format!("journal hash chain broken at sequence {}", event.sequence),
            ));
        }
        let recomputed = runtime::recompute_event_hash(event);
        if recomputed != event.hash {
            return Err(Diagnostic::new(
                DiagnosticCode::IkJournalCorrupt,
                Some("journal.hash"),
                format!("journal event hash mismatch at sequence {}", event.sequence),
            ));
        }
    }
    Ok(())
}

/// Verify that the stored receipt rows are exactly the receipts derived from the
/// events: same cardinality, same sequences, same content. Extra, missing,
/// duplicate, out-of-sequence, and unrelated-aggregate receipt rows all refuse;
/// the stored receipt set must equal the canonical journal event set.
fn verify_receipts(conn: &Connection, events: &[JournalEvent]) -> Result<(), Diagnostic> {
    let aggregate_id = &events[0].aggregate_id;
    let expected: Vec<Receipt> = events.iter().map(runtime::receipt_from_event).collect();
    let mut stmt = conn
        .prepare(
            "SELECT sequence, receipt_id, receipt_json FROM receipts WHERE aggregate_id = ?1 ORDER BY sequence",
        )
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map(params![aggregate_id.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(map_sqlite)?;
    let mut stored: Vec<(u64, Receipt)> = Vec::new();
    for row in rows {
        let (seq, receipt_id, json) = row.map_err(map_sqlite)?;
        let sequence = checked_u64(
            DiagnosticCode::IkProjectionCorrupt,
            "receipts.negative_sequence",
            "receipt sequence",
            seq,
        )?;
        let parsed: Receipt = serde_json::from_str(&json).map_err(|e| {
            Diagnostic::new(
                DiagnosticCode::IkProjectionCorrupt,
                Some("receipts.decode"),
                format!("stored receipt decode failed: {e}"),
            )
        })?;
        // The stored row columns and the receipt payload must agree; a disagreement
        // is projection corruption, even if the payload itself is well-formed.
        if parsed.sequence != sequence || parsed.receipt_id != receipt_id {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProjectionCorrupt,
                Some("receipts.columns"),
                format!(
                    "receipt row columns disagree with the receipt payload at sequence {sequence}"
                ),
            ));
        }
        stored.push((sequence, parsed));
    }
    // Extra (or duplicate, once constraints are absent) and missing rows both
    // change the cardinality; the stored set must equal the journal event set.
    if stored.len() != expected.len() {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProjectionCorrupt,
            Some("receipts.cardinality"),
            format!(
                "receipt set has {} rows but the journal has {} events for aggregate `{}`",
                stored.len(),
                expected.len(),
                aggregate_id.0,
            ),
        ));
    }
    // Both vectors are ordered by sequence; equal cardinality plus position-wise
    // equality means the sets are equal (sequences are unique per aggregate).
    for ((sequence, parsed), expected_receipt) in stored.iter().zip(&expected) {
        if parsed != expected_receipt {
            return Err(Diagnostic::new(
                DiagnosticCode::IkProjectionCorrupt,
                Some("receipts.mismatch"),
                format!("stored receipt at sequence {sequence} disagrees with the journal receipt"),
            ));
        }
    }
    Ok(())
}

/// Reject receipt rows keyed to an aggregate that no longer exists (or never
/// existed). Such rows are invisible to per-aggregate verification and would
/// otherwise survive as unrelated-aggregate projection residue.
fn verify_no_orphan_receipts(conn: &Connection) -> Result<(), Diagnostic> {
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM receipts WHERE aggregate_id NOT IN (SELECT aggregate_id FROM aggregates)",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite)?;
    if orphans > 0 {
        return Err(Diagnostic::new(
            DiagnosticCode::IkProjectionCorrupt,
            Some("receipts.orphan_aggregate"),
            format!("{orphans} receipt row(s) reference an unknown aggregate"),
        ));
    }
    Ok(())
}

/// Verify that the machine's initial state digest agrees with the genesis record
/// and return the deterministic initial state as the fold seed.
fn check_genesis_state(machine: &dyn Machine, event: &JournalEvent) -> Result<Value, Diagnostic> {
    let state = machine.initial_state();
    if digest_json(&state) != event.state_digest {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.genesis_state"),
            "machine initial state disagrees with the genesis record",
        ));
    }
    Ok(state)
}

/// The journal event binds the authoring machine identity; every event must agree
/// with the presented machine, not just the projection's recorded one.
fn check_machine_identity(machine: &dyn Machine, event: &JournalEvent) -> Result<(), Diagnostic> {
    if event.machine_id != machine.identity().machine_id
        || event.machine_version != machine.identity().version.0
        || event.machine_spec_hash != machine.identity().spec_hash.0
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.machine_identity"),
            format!(
                "event machine identity disagrees with the presented machine at sequence {}",
                event.sequence
            ),
        ));
    }
    Ok(())
}

/// Enforce the closed genesis contract on the first journal event: genesis kind and
/// type, version 1, the canonical payload binding aggregate and machine, no
/// effect/observation/note/terminality, and the exact `create` command.
fn check_genesis_contract(machine: &dyn Machine, event: &JournalEvent) -> Result<(), Diagnostic> {
    let expected_payload = serde_json::json!({
        "aggregate_id": event.aggregate_id,
        "machine": machine.identity(),
    });
    if event.event_kind != runtime::EventKind::Genesis
        || event.event_type != "genesis"
        || event.event_version != 1
        || event.payload != expected_payload
        || !matches!(event.effect_delta, EffectDelta::None)
        || event.observation.is_some()
        || event.note.is_some()
        || event.terminal
        || event.command.get("op").and_then(Value::as_str) != Some("create")
        || event.command.get("aggregate_id").and_then(Value::as_str)
            != Some(event.aggregate_id.0.as_str())
        || event.command.get("machine")
            != Some(
                &serde_json::to_value(machine.identity())
                    .expect("machine identity is JSON-serializable"),
            )
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.genesis"),
            "first journal event violates the closed genesis contract",
        ));
    }
    Ok(())
}

/// Fold a kernel-record event: the effect delta must be a Record whose payload
/// decodes to the recorded outcome, the accumulator must hold the matching
/// Pending intent, and the event must agree with the pending fold state. Returns
/// the advanced effect status.
fn replay_kernel_record(
    state: &Value,
    terminal: bool,
    effect: &EffectStatus,
    event: &JournalEvent,
) -> Result<EffectStatus, Diagnostic> {
    let EffectDelta::Record { outcome } = &event.effect_delta else {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.effect_delta"),
            format!(
                "kernel record lacks a record delta at sequence {}",
                event.sequence
            ),
        ));
    };
    let encoded: EffectOutcomeV1 =
        serde_json::from_value(event.payload.clone()).map_err(|error| {
            Diagnostic::new(
                DiagnosticCode::IkReplayMismatch,
                Some("replay.effect_payload"),
                format!(
                    "kernel record payload is not its effect outcome at sequence {}: {error}",
                    event.sequence
                ),
            )
        })?;
    let EffectStatus::Pending(intent) = &effect else {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.effect_status"),
            format!(
                "kernel record has no pending effect at sequence {}",
                event.sequence
            ),
        ));
    };
    if event.event_type != "kernel.effect_recorded"
        || event.event_version != 1
        || encoded != *outcome
        || !outcome.binds(intent)
        || event.observation.is_some()
        || event.state_digest != digest_json(state)
        || event.terminal != terminal
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.effect_record"),
            format!(
                "kernel record disagrees with its pending state at sequence {}",
                event.sequence
            ),
        ));
    }
    Ok(runtime::apply_effect_delta(
        effect,
        &event.effect_delta,
        &event.hash,
    ))
}

/// Fold a domain-command event through the machine, then validate the folded
/// state, terminality, and effect delta against the journal record. Returns the
/// advanced (state, terminality, effect status) tuple.
fn replay_domain_event(
    machine: &dyn Machine,
    state: &Value,
    effect: &EffectStatus,
    event: &JournalEvent,
) -> Result<(Value, bool, EffectStatus), Diagnostic> {
    let domain_event = crate::machine::DomainEvent {
        event_type: event.event_type.clone(),
        event_version: event.event_version,
        payload: event.payload.clone(),
    };
    #[cfg(feature = "test-support")]
    let domain_event =
        if crate::law_mutant::is(crate::law_mutant::LawMutant::RerunLiveGuardsOnReplay) {
            match machine.decide(state, None, effect, &event.command) {
                Ok(Decision::Emit {
                    event: live,
                    note: _,
                }) => live,
                _ => domain_event,
            }
        } else {
            domain_event
        };
    let applied = machine.apply(state, &domain_event).map_err(|diagnostic| {
        if diagnostic.code == DiagnosticCode::IkEventSchemaUnknown {
            diagnostic
        } else {
            Diagnostic::new(
                DiagnosticCode::IkReplayMismatch,
                Some("replay.apply"),
                format!(
                    "event fold refused at sequence {}: {diagnostic}",
                    event.sequence
                ),
            )
        }
    })?;
    if event.state_digest != digest_json(&applied.state) {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.state"),
            format!("folded state disagrees at sequence {}", event.sequence),
        ));
    }
    if applied.terminal != event.terminal {
        return Err(Diagnostic::new(
            DiagnosticCode::IkReplayMismatch,
            Some("replay.terminal"),
            format!(
                "folded terminality disagrees at sequence {}",
                event.sequence
            ),
        ));
    }
    let folded_outcome = applied
        .outcome
        .map(|value| runtime::stamp_outcome_aggregate(&value, &event.aggregate_id))
        .transpose()
        .map_err(|diagnostic| {
            Diagnostic::new(
                DiagnosticCode::IkReplayMismatch,
                Some("replay.effect_outcome"),
                format!(
                    "folded outcome could not bind at sequence {}: {diagnostic}",
                    event.sequence
                ),
            )
        })?;
    match (&event.effect_delta, applied.effect, folded_outcome) {
        (EffectDelta::None, None, None) => {}
        (EffectDelta::Plan { intent }, Some(emitted), None)
            // Live/replay parity (G-005): the live path (derive_effect_delta)
            // refuses a new plan only while one is PENDING; planning is legal
            // after a recorded outcome, so replay must accept a Plan delta from
            // both None and Recorded accumulator states.
            if !matches!(effect, EffectStatus::Pending(_))
                && runtime::intents_match_stamped(intent, &emitted) => {}
        (EffectDelta::Record { outcome }, None, Some(bound))
            if matches!(&effect, EffectStatus::Pending(intent)
                if outcome.binds(intent) && outcome == &bound) => {}
        _ => {
            return Err(Diagnostic::new(
                DiagnosticCode::IkReplayMismatch,
                Some("replay.effect_delta"),
                format!(
                    "folded effect delta disagrees at sequence {}",
                    event.sequence
                ),
            ))
        }
    }
    Ok((
        applied.state,
        applied.terminal,
        runtime::apply_effect_delta(effect, &event.effect_delta, &event.hash),
    ))
}

/// Replay events by folding versioned domain facts. Stored state, terminality, and
/// effect deltas are record-and-check fields, never replay authority.
fn replay_events(machine: &dyn Machine, events: &[JournalEvent]) -> Result<Replayed, Diagnostic> {
    let mut state = check_genesis_state(machine, &events[0])?;
    let mut effect = EffectStatus::None;
    let mut terminal = false;
    for (i, event) in events.iter().enumerate() {
        check_machine_identity(machine, event)?;
        if i == 0 {
            check_genesis_contract(machine, event)?;
            continue;
        }
        if event.event_kind == runtime::EventKind::KernelRecord {
            effect = replay_kernel_record(&state, terminal, &effect, event)?;
            continue;
        }
        if event.event_kind != runtime::EventKind::DomainCommand {
            return Err(Diagnostic::new(
                DiagnosticCode::IkReplayMismatch,
                Some("replay.event_kind"),
                format!(
                    "non-genesis event has invalid origin at sequence {}",
                    event.sequence
                ),
            ));
        }
        let (next_state, next_terminal, next_effect) =
            replay_domain_event(machine, &state, &effect, event)?;
        state = next_state;
        terminal = next_terminal;
        effect = next_effect;
    }
    let last = events.last().expect("non-empty journal");
    let nonce = runtime::derive_nonce(machine.identity(), &last.aggregate_id, last.generation);
    Ok(Replayed {
        state,
        terminal,
        generation: last.generation,
        nonce,
        effect,
        sequence: last.sequence,
        last_event_hash: last.hash.clone(),
    })
}

#[cfg(test)]
mod store_pragma_tests {
    use super::*;

    #[test]
    fn configure_defaults_to_synchronous_full_and_honors_normal_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::remove_var("IK_SQLITE_SYNCHRONOUS");
        let full_db = dir.path().join("full.db");
        let full_store = Store::open_or_create(&full_db).expect("open_or_create full");
        let full_sync: i64 = full_store
            .conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("query synchronous");
        // SQLite PRAGMA synchronous: 0=OFF, 1=NORMAL, 2=FULL, 3=EXTRA
        assert_eq!(full_sync, 2, "expected FULL (2) by default");
        drop(full_store);

        std::env::set_var("IK_SQLITE_SYNCHRONOUS", "normal");
        let norm_db = dir.path().join("normal.db");
        let norm_store = Store::open_or_create(&norm_db).expect("open_or_create normal");
        let norm_sync: i64 = norm_store
            .conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("query synchronous normal");
        std::env::remove_var("IK_SQLITE_SYNCHRONOUS");
        assert_eq!(norm_sync, 1, "expected NORMAL (1) when IK_SQLITE_SYNCHRONOUS=normal");
    }
}

struct Replayed {
    state: Value,
    terminal: bool,
    generation: u64,
    nonce: String,
    effect: EffectStatus,
    sequence: u64,
    last_event_hash: String,
}
