import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { parseWire, stringifyWire, type Json } from "./json.js";
import {
  EffectCompletionError,
  classifyError,
  KernelError,
  OperationConflictError,
  PreDispatchError,
  StaleCapabilityError,
  StoreNotFoundError,
  TransportError,
} from "./errors.js";
import type { FailureClass } from "./errors.js";
import { locateBinary } from "./locate.js";
import { unwrapEnvelope } from "./envelope.js";

export const PROTOCOL_MAJOR = 1;
export const PROTOCOL_MINOR = 0;
export const RUNTIME_VERSION = "0.6.0";
export type { Json };

export class MachineIdentity {
  constructor(
    readonly machine_id: string,
    readonly version: number | bigint,
    readonly spec_hash: string,
  ) {}

  static fromWire(data: Record<string, Json>): MachineIdentity {
    return new MachineIdentity(
      String(data.machine_id),
      data.version as number | bigint,
      String(data.spec_hash),
    );
  }

  toWire(): Record<string, Json> {
    return {
      machine_id: this.machine_id,
      version: this.version as Json,
      spec_hash: this.spec_hash,
    };
  }

  equals(other: MachineIdentity): boolean {
    return this.machine_id === other.machine_id
      && BigInt(this.version) === BigInt(other.version)
      && this.spec_hash === other.spec_hash;
  }
}

export class Capability {
  constructor(readonly raw: Record<string, Json>) {}
  toWire(): Record<string, Json> {
    return this.raw;
  }
  static fromWire(data: Record<string, Json>): Capability {
    return new Capability(data);
  }
}

/**
 * An exact repeat of an already committed operation. This is history, not authority: it
 * carries no capability and no state, and `current*` may be well past the commit. Re-read with
 * `machine.show` for a current handle.
 */
export class HistoricalResult {
  constructor(
    readonly aggregateId: string,
    readonly operationId: string,
    readonly kind: string,
    readonly firstSequence: number | bigint,
    readonly lastSequence: number | bigint,
    readonly generation: number | bigint,
    readonly eventHashes: string[],
    readonly receiptIds: string[],
    readonly currentGeneration: number | bigint,
    readonly currentSequence: number | bigint,
    readonly raw: Record<string, Json>,
  ) {}

  static fromHistory(
    aggregateId: string,
    record: Record<string, Json>,
    current: { generation: Json; sequence: Json },
    raw: Record<string, Json>,
  ): HistoricalResult {
    return new HistoricalResult(
      aggregateId,
      String(record.operation_id),
      String(record.kind),
      record.first_sequence as number | bigint,
      record.last_sequence as number | bigint,
      record.generation as number | bigint,
      record.event_hashes as string[],
      record.receipt_ids as string[],
      current.generation as number | bigint,
      current.sequence as number | bigint,
      raw,
    );
  }
}

/** Outcome of `lookupOperation`. `not_recorded` is not permission to retry under a stale handle. */
export class OperationLookup {
  constructor(
    readonly outcome: "committed_exact" | "conflict" | "not_recorded",
    readonly history: HistoricalResult | undefined,
    readonly currentGeneration: number | bigint | null,
    readonly currentSequence: number | bigint | null,
    readonly raw: Record<string, Json>,
  ) {}
}

/**
 * A recoverable description of one logical invocation, built BEFORE anything is sent: the
 * operation id and the original request (machine identity, aggregate, the capability the request
 * was authorized against, and the command or steps). Persist `toJson()` in the durable job record
 * you already keep, then `machine.dispatch(invocation)`. After a crash, `Invocation.fromJson` and
 * `machine.reconcile(invocation)` do an exact lookup of the original operation. Nothing here
 * refreshes a capability, derives an id from the current generation, or resends. If the
 * description is lost the invocation cannot be attributed from the store alone.
 *
 * An Invocation owns a snapshot: the request is serialized once at construction and every read
 * parses a fresh copy, so mutating a caller-owned argument object, or a value returned from here,
 * cannot change the bound request. A value that cannot be serialized (including an integer that
 * would lose precision) is rejected at construction, before anything is sent.
 */
export class Invocation {
  static readonly WIRE_VERSION = 1;
  readonly #json: string;

  constructor(
    kind: "command" | "transaction",
    machine: MachineIdentity,
    aggregateId: string,
    operationId: string,
    capability: Capability,
    body: OperationBody,
  ) {
    if (operationId === "") throw new Error("an invocation needs a non-empty operation id");
    if (kind === "command" && (body.command === undefined || body.steps !== undefined)) {
      throw new Error("a command invocation carries a command and no steps");
    }
    if (kind === "transaction" && (body.steps === undefined || body.command !== undefined)) {
      throw new Error("a transaction invocation carries steps and no command");
    }
    const wire: Record<string, Json> = {
      invocation_version: Invocation.WIRE_VERSION,
      kind,
      machine: machine.toWire(),
      aggregate_id: aggregateId,
      operation_id: operationId,
      capability: capability.toWire(),
    };
    if (body.command !== undefined) wire.command = body.command;
    if (body.observation !== undefined) wire.observation = body.observation;
    if (body.steps !== undefined) wire.steps = body.steps;
    this.#json = stringifyWire(sortKeys(wire));
  }

  private wire(): Record<string, Json> {
    return parseWire(this.#json) as Record<string, Json>;
  }

  get kind(): "command" | "transaction" {
    return this.wire().kind as "command" | "transaction";
  }
  get machine(): MachineIdentity {
    return MachineIdentity.fromWire(this.wire().machine as Record<string, Json>);
  }
  get aggregateId(): string {
    return this.wire().aggregate_id as string;
  }
  get operationId(): string {
    return this.wire().operation_id as string;
  }
  get capability(): Capability {
    return Capability.fromWire(this.wire().capability as Record<string, Json>);
  }
  /** A fresh copy of the bound request body. */
  get body(): OperationBody {
    const wire = this.wire();
    const body: OperationBody = {};
    if (wire.command !== undefined) body.command = wire.command as Record<string, Json>;
    if (wire.observation !== undefined) body.observation = wire.observation as Record<string, Json>;
    if (wire.steps !== undefined) body.steps = wire.steps as Record<string, Json>[];
    return body;
  }

  /** A fresh copy for the host's durable record; mutating it changes nothing. */
  toWire(): Record<string, Json> {
    return this.wire();
  }

  toJson(): string {
    return this.#json;
  }

  static fromWire(data: Record<string, Json>): Invocation {
    if (data.invocation_version !== Invocation.WIRE_VERSION) {
      throw new Error("unknown invocation_version");
    }
    const known: Record<string, true> = {
      invocation_version: true, kind: true, machine: true, aggregate_id: true, operation_id: true,
      capability: true, command: true, observation: true, steps: true,
    };
    const extra = Object.keys(data).filter((key) => known[key] !== true);
    if (extra.length > 0) throw new Error(`unknown invocation fields: ${extra.sort().join(", ")}`);
    if (data.kind !== "command" && data.kind !== "transaction") {
      throw new Error("invocation kind must be command or transaction");
    }
    const body: OperationBody = {};
    if (data.command !== undefined) body.command = data.command as Record<string, Json>;
    if (data.observation !== undefined) body.observation = data.observation as Record<string, Json>;
    if (data.steps !== undefined) body.steps = data.steps as Record<string, Json>[];
    return new Invocation(
      data.kind,
      MachineIdentity.fromWire(data.machine as Record<string, Json>),
      String(data.aggregate_id),
      String(data.operation_id),
      Capability.fromWire(data.capability as Record<string, Json>),
      body,
    );
  }

  static fromJson(text: string): Invocation {
    return Invocation.fromWire(parseWire(text) as Record<string, Json>);
  }
}

function sortKeys(value: Json): Json {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === "object") {
    const out: Record<string, Json> = {};
    for (const key of Object.keys(value).sort()) {
      out[key] = sortKeys((value as Record<string, Json>)[key]);
    }
    return out;
  }
  return value;
}

export interface OperationBody {
  command?: Record<string, Json>;
  observation?: Record<string, Json>;
  steps?: Record<string, Json>[];
  effect?: Record<string, Json>;
}

export interface CallFields extends OperationBody {
  aggregate_id?: string;
  capability?: Capability;
  operation_id?: string;
}

/** Exact recorded effect identity. `generation` is the planned generation, not a later current one. */
export class EffectIntent {
  constructor(
    readonly effectId: string,
    readonly aggregateId: string,
    readonly generation: number | bigint,
    readonly effectKind: string,
    readonly payloadDigest: string,
    readonly idempotencyKey: string,
    readonly payload: Json | undefined,
  ) {}

  static fromWire(raw: Record<string, Json>): EffectIntent {
    return new EffectIntent(
      String(raw.effect_id),
      String(raw.aggregate_id),
      raw.generation as number | bigint,
      String(raw.effect_kind ?? ""),
      String(raw.payload_digest),
      String(raw.idempotency_key),
      raw.payload,
    );
  }

  toWire(): Record<string, Json> {
    const wire: Record<string, Json> = {
      effect_id: this.effectId,
      aggregate_id: this.aggregateId,
      generation: this.generation as Json,
      effect_kind: this.effectKind,
      payload_digest: this.payloadDigest,
      idempotency_key: this.idempotencyKey,
    };
    if (this.payload !== undefined) {
      wire.payload = this.payload;
    }
    return wire;
  }

  sameBinding(other: EffectIntent): boolean {
    return (
      this.effectId === other.effectId &&
      this.aggregateId === other.aggregateId &&
      String(this.generation) === String(other.generation) &&
      this.effectKind === other.effectKind &&
      this.payloadDigest === other.payloadDigest &&
      this.idempotencyKey === other.idempotencyKey
    );
  }
}

/** Runtime effect status: `none`, `pending`, or `recorded`. */
export class Effect {
  constructor(
    readonly status: string,
    readonly intent: EffectIntent | undefined,
    readonly outcomeDigest: string | undefined,
    readonly receiptId: string | undefined,
  ) {}

  static none(): Effect {
    return new Effect("none", undefined, undefined, undefined);
  }

  static fromWire(raw: Json | undefined): Effect {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      return Effect.none();
    }
    const rec = raw as Record<string, Json>;
    const intentRaw = rec.intent;
    const intent =
      intentRaw && typeof intentRaw === "object" && !Array.isArray(intentRaw)
        ? EffectIntent.fromWire(intentRaw as Record<string, Json>)
        : undefined;
    return new Effect(
      String(rec.status ?? "none"),
      intent,
      typeof rec.outcome_digest === "string" ? rec.outcome_digest : undefined,
      typeof rec.receipt_id === "string" ? rec.receipt_id : undefined,
    );
  }
}

function outcomeWire(intent: EffectIntent, outcomeDigest: string): Record<string, Json> {
  return {
    effect_id: intent.effectId,
    aggregate_id: intent.aggregateId,
    planned_generation: intent.generation as Json,
    payload_digest: intent.payloadDigest,
    idempotency_key: intent.idempotencyKey,
    outcome_digest: outcomeDigest,
  };
}

export interface EventsOptions {
  since?: number | bigint;
  aggregateId?: string;
  limit?: number;
}

export interface EventProjection {
  state: Json;
  sequence: number | bigint;
  generation: number | bigint;
  terminal: boolean;
}

export interface EventCursorRow {
  cursor: number | bigint;
  aggregateId: string;
  sequence: number | bigint;
  generation: number | bigint;
  machineId: string;
  machineVersion: number | bigint;
  machineSpecHash: string;
  eventKind: string;
  eventType: string;
  eventVersion: number | bigint;
  command: Json;
  stateDigest: string;
  terminal: boolean;
  effectDelta: Json;
  eventHash: string;
  prevEventHash: string;
  isLatestForAggregate: boolean;
  currentProjection: EventProjection;
  raw: Record<string, Json>;
}

export interface EventFeed {
  schema: string;
  store: string;
  since: number | bigint;
  nextCursor: number | bigint;
  hasMore: boolean;
  events: EventCursorRow[];
  raw: Record<string, Json>;
}

export interface EffectRunResult<T = undefined> {
  planned: Aggregate;
  recorded: Aggregate | HistoricalResult;
  intent: EffectIntent;
  outcomeDigest: string;
  result?: T;
}

export class Aggregate {
  constructor(
    readonly machine: Machine,
    readonly id: string,
    readonly capability: Capability,
    readonly state: Json,
    readonly generation: number | bigint,
    readonly terminal: boolean,
    readonly raw: Record<string, Json>,
    readonly mutation: boolean | undefined,
    readonly effect: Effect = Effect.none(),
  ) {}

  static fromResult(machine: Machine, result: Record<string, Json>): Aggregate {
    const cap = result.capability;
    if (!cap || typeof cap !== "object" || Array.isArray(cap)) {
      throw new TransportError("result missing capability");
    }
    return new Aggregate(
      machine,
      String(result.aggregate_id ?? ""),
      new Capability(cap as Record<string, Json>),
      result.state as Json,
      result.generation as number | bigint,
      Boolean(result.terminal),
      result,
      typeof result.mutation === "boolean" ? result.mutation : undefined,
      Effect.fromWire(result.effect),
    );
  }

  /** The pending effect intent, or undefined when nothing is pending. */
  get pendingIntent(): EffectIntent | undefined {
    if (this.effect.status !== "pending") {
      return undefined;
    }
    return this.effect.intent;
  }

  namedState(): string {
    if (this.state && typeof this.state === "object" && !Array.isArray(this.state) && "state" in this.state) {
      return String((this.state as Record<string, Json>).state);
    }
    return String(this.state);
  }

  /**
   * The identity that owns this aggregate in the store. After a machine version change it can differ from
   * `machine.identity` on a `show` through another version. Route an existing aggregate to the retained
   * definition whose `identity.equals(owner)`.
   */
  get owner(): MachineIdentity {
    const wire = this.raw.machine;
    if (wire && typeof wire === "object" && !Array.isArray(wire)) {
      return MachineIdentity.fromWire(wire as Record<string, Json>);
    }
    return this.machine.identity;
  }

  /** Invoke a command under a freshly generated operation id; see `execute` to supply one. */
  async command(name: string, args?: Record<string, Json>): Promise<Aggregate> {
    const command: Record<string, Json> = { name };
    if (args) {
      command.args = args;
    }
    const settled = await this.execute(command);
    if (settled instanceof HistoricalResult) {
      throw new TransportError("a freshly generated operation id answered as history");
    }
    return settled;
  }

  /**
   * Dispatch one command exactly once under `operationId` (generated when omitted). There is no
   * automatic retry or handle refresh. A failure carries `error.operationId`; reconcile with
   * `machine.lookupOperation`. An exact repeat of a committed operation resolves to a
   * `HistoricalResult` (history, not a usable handle).
   */
  execute(
    command: Record<string, Json>,
    options: { operationId?: string; observation?: Record<string, Json> } = {},
  ): Promise<Aggregate | HistoricalResult> {
    return this.dispatch("command", options.operationId, {
      command,
      observation: options.observation,
    });
  }

  /**
   * Describe one command invocation BEFORE sending it. Nothing is sent. Persist
   * `invocation.toJson()` in your durable job record, then `machine.dispatch(invocation)`.
   */
  prepare(
    command: Record<string, Json>,
    options: { operationId?: string; observation?: Record<string, Json> } = {},
  ): Invocation {
    return new Invocation(
      "command",
      this.machine.identity,
      this.id,
      options.operationId ?? this.machine.kernel.newOperationId(),
      this.capability,
      { command, observation: options.observation },
    );
  }

  /** Describe an atomic transaction BEFORE sending it; see `prepare`. */
  prepareTransaction(
    steps: Record<string, Json>[],
    options: { operationId?: string } = {},
  ): Invocation {
    return new Invocation(
      "transaction",
      this.machine.identity,
      this.id,
      options.operationId ?? this.machine.kernel.newOperationId(),
      this.capability,
      { steps },
    );
  }

  /** All-or-nothing ordered commands in one store transaction, bound to one operation id. */
  transaction(
    steps: Record<string, Json>[],
    options: { operationId?: string } = {},
  ): Promise<Aggregate | HistoricalResult> {
    return this.dispatch("transaction", options.operationId, { steps });
  }

  /** Bind an external effect outcome to the pending effect identity. */
  record(
    effect: Record<string, Json>,
    options: { operationId?: string } = {},
  ): Promise<Aggregate | HistoricalResult> {
    return this.dispatch("record", options.operationId, { effect });
  }

  /**
   * Bind `outcomeDigest` to this aggregate's pending intent. Identity fields are
   * copied from that intent. The digest is caller-supplied. Dispatch is exactly
   * once; a transport failure stays submission-unknown and is not retried.
   * This method does not perform external I/O.
   */
  recordEffectOutcome(
    outcomeDigest: string,
    options: { operationId?: string; intent?: EffectIntent } = {},
  ): Promise<Aggregate | HistoricalResult> {
    let pending: EffectIntent;
    try {
      pending = completionIntent(this.effect, outcomeDigest, options.intent);
    } catch (err) {
      if (err instanceof EffectCompletionError && options.operationId) {
        err.operationId = options.operationId;
      }
      throw err;
    }
    return this.dispatch("record", options.operationId, {
      effect: outcomeWire(pending, outcomeDigest),
    });
  }

  /**
   * Plan an external effect, execute `executor(intent)` once with the kernel-bound
   * `EffectIntent` (including `idempotency_key`), and record the returned outcome digest.
   *
   * Picky and explicit: refuses if the planning command replays as `HistoricalResult`
   * (so callers do not blindly re-run side effects on duplicate plans) or does not plan
   * an effect. Never silently retries transport failures.
   */
  async runEffect<T = undefined>(
    commandName: string,
    args: Record<string, Json>,
    executor: (
      intent: EffectIntent,
    ) => Promise<string | { outcomeDigest: string; result?: T }> | string | { outcomeDigest: string; result?: T },
    options: { planOperationId?: string; recordOperationId?: string } = {},
  ): Promise<EffectRunResult<T>> {
    const planned = await this.execute(
      { name: commandName, args },
      { operationId: options.planOperationId },
    );
    if (planned instanceof HistoricalResult) {
      const err = new EffectCompletionError(
        `planning command \`${commandName}\` replayed as HistoricalResult; inspect show() and reconcile before re-running external side effects`,
      );
      err.operationId = options.planOperationId;
      throw err;
    }
    if (planned.effect.status !== "pending" || !planned.effect.intent) {
      const err = new EffectCompletionError(
        `command \`${commandName}\` committed without planning a pending effect intent`,
      );
      err.operationId = options.planOperationId;
      throw err;
    }
    const intent = planned.effect.intent;
    const executed = await executor(intent);
    const outcomeDigest = typeof executed === "string" ? executed : executed.outcomeDigest;
    const result = typeof executed === "string" ? undefined : executed.result;
    const recorded = await planned.recordEffectOutcome(outcomeDigest, {
      operationId: options.recordOperationId,
      intent,
    });
    return { planned, recorded, intent, outcomeDigest, result };
  }

  private async dispatch(
    op: string,
    operationId: string | undefined,
    body: OperationBody,
  ): Promise<Aggregate | HistoricalResult> {
    const result = await this.machine.kernel.callBound(op, this.machine, operationId, {
      aggregate_id: this.id,
      capability: this.capability,
      ...body,
    });
    if (result.historical === true) {
      const current = result.current as { generation: Json; sequence: Json };
      return HistoricalResult.fromHistory(
        this.id,
        result.operation as Record<string, Json>,
        current,
        result,
      );
    }
    return Aggregate.fromResult(this.machine, result);
  }
}

export class Machine {
  constructor(
    readonly kernel: Kernel,
    readonly path: string,
    readonly identity: MachineIdentity,
  ) {}

  async create(aggregateId: string): Promise<Aggregate> {
    const result = await this.kernel.call("create", this, { aggregate_id: aggregateId });
    return Aggregate.fromResult(this, result);
  }

  /**
   * Create the aggregate and apply the initializing command in one validated operation.
   * An invalid command or a law violation exposes no aggregate. The same operation id
   * replays as history. A dropped reply is reconciled with lookupCreateWith.
   * This is not create followed by a later command.
   */
  async createWith(input: {
    aggregateId: string;
    command: Record<string, Json>;
    operationId?: string;
    observation?: Record<string, Json>;
  }): Promise<Aggregate | HistoricalResult> {
    const result = await this.kernel.callBound("create_with_command", this, input.operationId, {
      aggregate_id: input.aggregateId,
      command: input.command,
      observation: input.observation,
    });
    if (result.historical === true) {
      const current = result.current as { generation: Json; sequence: Json };
      return HistoricalResult.fromHistory(
        input.aggregateId,
        result.operation as Record<string, Json>,
        current,
        result,
      );
    }
    return Aggregate.fromResult(this, result);
  }

  /** Ask whether a createWith operation id committed. Do not send a capability. */
  async lookupCreateWith(
    aggregateId: string,
    operationId: string,
    command: Record<string, Json>,
    observation?: Record<string, Json>,
  ): Promise<OperationLookup> {
    const result = await this.kernel.call("lookup_operation", this, {
      aggregate_id: aggregateId,
      operation_id: operationId,
      command,
      observation,
    });
    return lookupFrom(aggregateId, result);
  }

  async show(aggregateId: string): Promise<Aggregate> {
    const result = await this.kernel.call("show", this, { aggregate_id: aggregateId });
    return Aggregate.fromResult(this, result);
  }

  /** Journal audits. These do not answer whether an operation committed; use `lookupOperation`. */
  replay(aggregateId?: string): Promise<Record<string, Json>> {
    return this.kernel.call("replay", this, { aggregate_id: aggregateId });
  }

  verify(aggregateId?: string): Promise<Record<string, Json>> {
    return this.kernel.call("verify", this, { aggregate_id: aggregateId });
  }

  /**
   * Ask, without mutating anything, whether `operationId` committed. Pass the ORIGINAL request
   * body and the capability the original attempt used; a changed body reports `conflict`.
   */
  async lookupOperation(
    aggregateId: string,
    capability: Capability,
    operationId: string,
    body: OperationBody,
  ): Promise<OperationLookup> {
    const result = await this.kernel.call("lookup_operation", this, {
      aggregate_id: aggregateId,
      capability,
      operation_id: operationId,
      ...body,
    });
    return lookupFrom(aggregateId, result);
  }

  /**
   * Send a prepared invocation exactly once under its own operation id and capability. No retry
   * and no handle refresh. On failure, `reconcile` with the same invocation; never resend under
   * a new id.
   */
  async dispatch(invocation: Invocation): Promise<Aggregate | HistoricalResult> {
    this.checkInvocation(invocation);
    const result = await this.kernel.callBound(
      invocation.kind,
      this,
      invocation.operationId,
      {
        aggregate_id: invocation.aggregateId,
        capability: invocation.capability,
        ...invocation.body,
      },
    );
    if (result.historical === true) {
      const current = result.current as { generation: Json; sequence: Json };
      return HistoricalResult.fromHistory(
        invocation.aggregateId,
        result.operation as Record<string, Json>,
        current,
        result,
      );
    }
    return Aggregate.fromResult(this, result);
  }

  /**
   * Exact lookup of a prepared invocation: did THIS operation id commit under THIS request?
   * `committed_exact` is history, not a current handle; `conflict` means the id was used for a
   * different request; `not_recorded` is not permission to resend under a refreshed handle.
   */
  async reconcile(invocation: Invocation): Promise<OperationLookup> {
    this.checkInvocation(invocation);
    return this.lookupOperation(
      invocation.aggregateId,
      invocation.capability,
      invocation.operationId,
      invocation.body,
    );
  }

  private checkInvocation(invocation: Invocation): void {
    if (!(invocation instanceof Invocation)) throw new TypeError("expected an Invocation");
    if (!invocation.machine.equals(this.identity)) {
      throw new Error(
        "this invocation was prepared for a different machine identity than this Machine",
      );
    }
  }
}

function lookupFrom(aggregateId: string, result: Record<string, Json>): OperationLookup {
  const outcome = result.outcome as OperationLookup["outcome"];
  const history =
    outcome === "committed_exact"
      ? HistoricalResult.fromHistory(
          aggregateId,
          result.record as Record<string, Json>,
          { generation: result.current_generation, sequence: result.current_sequence },
          result,
        )
      : undefined;
  return new OperationLookup(
    outcome,
    history,
    (result.current_generation ?? null) as number | bigint | null,
    (result.current_sequence ?? null) as number | bigint | null,
    result,
  );
}

const MAX_OUTPUT_BYTES = 1024 * 1024;

class PersistentSession {
  private readonly child: ReturnType<typeof spawn>;
  private readonly queue: Array<{
    requestId: string;
    op: string;
    line: string;
    timeoutMs: number;
    resolve: (value: unknown) => void;
    reject: (err: Error) => void;
  }> = [];
  private active: {
    requestId: string;
    op: string;
    timer: ReturnType<typeof setTimeout>;
    resolve: (value: unknown) => void;
    reject: (err: Error) => void;
  } | null = null;
  private stdoutBuf = "";
  private stderrBuf = "";
  private broken: string | null = null;
  private closed = false;

  constructor(binary: string, store: string, machinePath?: string) {
    const argv = ["--store", store];
    if (machinePath) {
      argv.push("--machine", machinePath);
    }
    argv.push("--session");
    this.child = spawn(binary, argv, { stdio: ["pipe", "pipe", "pipe"] });
    this.setIdleUnref(true);

    this.child.stdout?.on("data", (chunk: Buffer) => {
      if (this.stdoutBuf.length + chunk.length > MAX_OUTPUT_BYTES) {
        this.fail(`ik session output exceeded ${MAX_OUTPUT_BYTES} bytes`);
        return;
      }
      this.stdoutBuf += chunk.toString("utf8");
      let nl = this.stdoutBuf.indexOf("\n");
      while (nl !== -1) {
        const line = this.stdoutBuf.slice(0, nl);
        this.stdoutBuf = this.stdoutBuf.slice(nl + 1);
        this.onLine(line);
        nl = this.stdoutBuf.indexOf("\n");
      }
    });

    this.child.stderr?.on("data", (chunk: Buffer) => {
      if (this.stderrBuf.length < MAX_OUTPUT_BYTES) {
        this.stderrBuf += chunk.toString("utf8").slice(0, MAX_OUTPUT_BYTES - this.stderrBuf.length);
      }
    });

    this.child.on("error", (err: NodeJS.ErrnoException) => {
      const neverStarted = err.code === "ENOENT" || err.code === "EACCES" || err.code === "EPERM";
      this.fail(
        neverStarted ? `ik binary not found: ${err.message}` : `ik session error: ${err.message}`,
        neverStarted,
      );
    });

    this.child.on("close", (code) => {
      if (!this.closed) {
        const detail = this.stderrBuf.trim();
        this.fail(detail || `ik session exited unexpectedly (code ${code ?? "null"})`);
      }
    });

    this.child.stdin?.on("error", (err: NodeJS.ErrnoException) => {
      if (!this.closed) {
        this.fail(`ik session stdin error: ${err.message}`);
      }
    });
  }

  private setIdleUnref(idle: boolean): void {
    const toggle = (h: unknown) => {
      const target = h as { ref?: () => void; unref?: () => void } | null | undefined;
      if (idle) {
        target?.unref?.();
      } else {
        target?.ref?.();
      }
    };
    toggle(this.child);
    toggle(this.child.stdin);
    toggle(this.child.stdout);
    toggle(this.child.stderr);
  }

  isBroken(): boolean {
    return this.broken !== null || this.closed;
  }

  send(requestId: string, op: string, line: string, timeoutMs: number): Promise<unknown> {
    if (this.broken !== null) {
      return Promise.reject(new PreDispatchError(`ik session unusable: ${this.broken}`));
    }
    const { promise, resolve, reject } = Promise.withResolvers<unknown>();
    this.queue.push({ requestId, op, line, timeoutMs, resolve, reject });
    this.pump();
    return promise;
  }

  private pump(): void {
    if (this.active !== null || this.queue.length === 0) {
      if (this.active === null) {
        this.setIdleUnref(true);
      }
      return;
    }
    if (this.broken !== null) {
      while (this.queue.length > 0) {
        this.queue.shift()!.reject(new PreDispatchError(`ik session unusable: ${this.broken}`));
      }
      return;
    }
    const next = this.queue.shift()!;
    this.setIdleUnref(false);
    const timer = setTimeout(() => {
      this.fail(`ik session ${next.op} timed out after ${next.timeoutMs}ms`);
    }, next.timeoutMs);
    this.active = {
      requestId: next.requestId,
      op: next.op,
      timer,
      resolve: next.resolve,
      reject: next.reject,
    };
    this.child.stdin?.write(`${next.line}\n`);
  }

  private onLine(line: string): void {
    const cur = this.active;
    if (!cur) {
      return;
    }
    clearTimeout(cur.timer);
    this.active = null;
    try {
      cur.resolve(parseWire(line));
    } catch (err) {
      this.fail(`ik session returned non-JSON for ${cur.op}: ${String(err)}`);
      cur.reject(new TransportError(`ik session returned non-JSON for ${cur.op}: ${String(err)}`));
      return;
    }
    this.pump();
  }

  private fail(reason: string, preDispatch = false): void {
    if (this.broken !== null) {
      return;
    }
    this.broken = reason;
    const cur = this.active;
    this.active = null;
    if (cur) {
      clearTimeout(cur.timer);
      cur.reject(preDispatch ? new PreDispatchError(reason) : new TransportError(reason));
    }
    while (this.queue.length > 0) {
      this.queue.shift()!.reject(new PreDispatchError(`ik session unusable: ${reason}`));
    }
    this.child.kill("SIGKILL");
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.broken = "closed";
    if (this.active) {
      clearTimeout(this.active.timer);
      this.active.reject(new TransportError("ik session closed while request was in flight"));
      this.active = null;
    }
    while (this.queue.length > 0) {
      this.queue.shift()!.reject(new PreDispatchError("ik session closed"));
    }
    try {
      this.child.stdin?.end();
    } catch {
      // ignore
    }
    this.child.kill("SIGTERM");
  }
}

export class Kernel {
  readonly binary: string;
  readonly store: string;
  readonly timeoutMs: number;
  readonly useSession: boolean;
  runtimeVersion = RUNTIME_VERSION;
  protocol = { major: PROTOCOL_MAJOR, minor: PROTOCOL_MINOR };
  private nextId = 1;
  private readonly sessions = new Map<string, PersistentSession>();

  constructor(
    store: string,
    options: { binary?: string; timeoutMs?: number; session?: boolean } = {},
  ) {
    this.store = store;
    this.binary = locateBinary(options.binary);
    this.timeoutMs = options.timeoutMs ?? 30_000;
    this.useSession = options.session ?? true;
  }

  /** Close any open persistent `ik --session` processes. Safe to call repeatedly. */
  close(): void {
    for (const session of this.sessions.values()) {
      session.close();
    }
    this.sessions.clear();
  }

  private sessionFor(machine?: Machine): PersistentSession {
    const key = machine?.path ?? "";
    const existing = this.sessions.get(key);
    if (existing && !existing.isBroken()) {
      return existing;
    }
    const created = new PersistentSession(this.binary, this.store, machine?.path);
    this.sessions.set(key, created);
    return created;
  }

  /** A fresh caller operation id. Persist it before dispatch to survive your own crash. */
  newOperationId(): string {
    return `ts-${randomUUID()}`;
  }

  machine(path: string): Promise<Machine> {
    return this.loadMachine(path);
  }

  /** Stream ordered events after `since` cursor with each aggregate's latest projection. */
  async events(options: EventsOptions = {}): Promise<EventFeed> {
    const argv = ["events", this.store, "--since", String(options.since ?? 0)];
    if (options.aggregateId !== undefined) {
      argv.push("--aggregate", options.aggregateId);
    }
    if (options.limit !== undefined) {
      argv.push("--limit", String(options.limit));
    }
    argv.push("--json");
    const data = await runJson(this.binary, argv, this.timeoutMs);
    if (!data || typeof data !== "object" || Array.isArray(data)) {
      throw new TransportError("ik events returned a non-object");
    }
    const rec = data as Record<string, Json>;
    if (rec.schema !== "ik.events.v1" || !Array.isArray(rec.events)) {
      throw new TransportError("ik events returned an unexpected schema");
    }
    const events: EventCursorRow[] = rec.events.map((item) => {
      const row = item as Record<string, Json>;
      const proj = row.current_projection as Record<string, Json>;
      return {
        cursor: row.cursor as number | bigint,
        aggregateId: String(row.aggregate_id ?? ""),
        sequence: row.sequence as number | bigint,
        generation: row.generation as number | bigint,
        machineId: String(row.machine_id ?? ""),
        machineVersion: row.machine_version as number | bigint,
        machineSpecHash: String(row.machine_spec_hash ?? ""),
        eventKind: String(row.event_kind ?? ""),
        eventType: String(row.event_type ?? ""),
        eventVersion: row.event_version as number | bigint,
        command: row.command as Json,
        stateDigest: String(row.state_digest ?? ""),
        terminal: Boolean(row.terminal),
        effectDelta: row.effect_delta as Json,
        eventHash: String(row.event_hash ?? ""),
        prevEventHash: String(row.prev_event_hash ?? ""),
        isLatestForAggregate: Boolean(row.is_latest_for_aggregate),
        currentProjection: {
          state: proj.state as Json,
          sequence: proj.sequence as number | bigint,
          generation: proj.generation as number | bigint,
          terminal: Boolean(proj.terminal),
        },
        raw: row,
      };
    });
    return {
      schema: String(rec.schema),
      store: String(rec.store ?? ""),
      since: rec.since as number | bigint,
      nextCursor: rec.next_cursor as number | bigint,
      hasMore: Boolean(rec.has_more),
      events,
      raw: rec,
    };
  }

  private async loadMachine(path: string): Promise<Machine> {
    const data = await runJson(this.binary, ["identity", path], this.timeoutMs);
    if (!data || typeof data !== "object" || Array.isArray(data)) {
      throw new TransportError("ik identity returned a non-object");
    }
    return new Machine(this, path, MachineIdentity.fromWire(data as Record<string, Json>));
  }

  /** Dispatch once under `operationId` (generated when omitted); never retries. Failures carry the id. */
  async callBound(
    op: string,
    machine: Machine,
    operationId: string | undefined,
    fields: CallFields,
  ): Promise<Record<string, Json>> {
    const bound = operationId ?? this.newOperationId();
    try {
      return await this.call(op, machine, { ...fields, operation_id: bound });
    } catch (err) {
      if (err instanceof KernelError) {
        err.operationId = bound;
      }
      throw err;
    }
  }

  async call(
    op: string,
    machine: Machine | undefined,
    fields: CallFields,
  ): Promise<Record<string, Json>> {
    const requestId = `ts-${this.nextId++}`;
    const env: Record<string, Json> = {
      protocol: { major: this.protocol.major, minor: this.protocol.minor },
      runtime_version: this.runtimeVersion,
      request_id: requestId,
      op,
    };
    if (fields.aggregate_id !== undefined) {
      env.aggregate_id = fields.aggregate_id;
    }
    if (machine) {
      env.machine = machine.identity.toWire();
    }
    if (fields.capability) {
      env.capability = fields.capability.toWire();
    }
    if (fields.command) {
      env.command = fields.command;
    }
    if (fields.observation) {
      env.observation = fields.observation;
    }
    if (fields.steps) {
      env.commands = fields.steps;
    }
    if (fields.effect) {
      env.effect = fields.effect;
    }
    if (fields.operation_id !== undefined) {
      env.operation_id = fields.operation_id;
    }
    const envelope = this.useSession
      ? await this.sessionFor(machine).send(requestId, op, stringifyWire(env), this.timeoutMs)
      : await (() => {
          const argv = [this.binary, "--store", this.store];
          if (machine) {
            argv.push("--machine", machine.path);
          }
          argv.push(op);
          return runJson(argv[0], argv.slice(1), this.timeoutMs, stringifyWire(env));
        })();
    return unwrapEnvelope(envelope, {
      request_id: requestId,
      runtime_version: this.runtimeVersion,
      protocol: this.protocol,
    });
  }
}


function runJson(
  binary: string,
  argv: string[],
  timeoutMs: number,
  stdinText?: string,
): Promise<unknown> {
  const { promise, resolve, reject } = Promise.withResolvers<unknown>();
  const child = spawn(binary, argv, { stdio: ["pipe", "pipe", "pipe"] });
  const chunks: Buffer[] = [];
  const errChunks: Buffer[] = [];
  let outBytes = 0;
  let errBytes = 0;
  let overflowed = false;
  const timer = setTimeout(() => {
    child.kill("SIGKILL");
    reject(new TransportError(`ik timed out after ${timeoutMs}ms`));
  }, timeoutMs);
  child.stdout.on("data", (c: Buffer) => {
    outBytes += c.length;
    if (outBytes <= MAX_OUTPUT_BYTES) {
      chunks.push(c);
    } else if (!overflowed) {
      overflowed = true;
      child.kill("SIGKILL");
    }
  });
  child.stderr.on("data", (c: Buffer) => {
    if (errBytes < MAX_OUTPUT_BYTES) {
      errChunks.push(c.subarray(0, MAX_OUTPUT_BYTES - errBytes));
      errBytes += c.length;
    }
  });
  child.on("error", (err: NodeJS.ErrnoException) => {
    clearTimeout(timer);
    const neverStarted = err.code === "ENOENT" || err.code === "EACCES" || err.code === "EPERM";
    reject(
      neverStarted
        ? new PreDispatchError(`ik binary not found: ${err.message}`)
        : new TransportError(`ik process error: ${err.message}`),
    );
  });
  child.on("close", (code) => {
    clearTimeout(timer);
    if (overflowed) {
      reject(new TransportError(`ik stdout exceeded ${MAX_OUTPUT_BYTES} bytes`));
      return;
    }
    const stdout = Buffer.concat(chunks).toString("utf8");
    const stderr = Buffer.concat(errChunks).toString("utf8");
    if (code === 2 && !stdout.trim()) {
      reject(new TransportError(stderr.trim() || `ik exited 2`));
      return;
    }
    try {
      resolve(parseWire(stdout));
    } catch (err) {
      reject(
        new TransportError(
          `ik returned non-JSON (exit ${code}): ${stdout || stderr} (${String(err)})`,
        ),
      );
    }
  });
  // A child that exits without reading stdin (for example `ik identity`) closes the pipe: EPIPE
  // is expected there. The child's stdout and exit status, handled on "close", are authoritative.
  child.stdin.on("error", (err: NodeJS.ErrnoException) => {
    if (err.code !== "EPIPE") {
      reject(new TransportError(`ik stdin error: ${err.message}`));
    }
  });
  child.stdin.end(stdinText ?? "");
  return promise;
}

function completionIntent(
  effect: Effect,
  outcomeDigest: string,
  supplied: EffectIntent | undefined,
): EffectIntent {
  if (typeof outcomeDigest !== "string" || outcomeDigest.length === 0) {
    throw new EffectCompletionError("outcome digest is required and must be non-empty");
  }
  const pending = effect.status === "pending" ? effect.intent : undefined;
  if (!pending) {
    throw new EffectCompletionError("no pending effect intent to bind");
  }
  if (supplied && !pending.sameBinding(supplied)) {
    throw new EffectCompletionError("intent is not this aggregate's pending effect intent");
  }
  return pending;
}

/** Classify a caught error, or a historical result. `historical` is not an error. */
export function classify(value: KernelError | HistoricalResult): FailureClass {
  if (value instanceof HistoricalResult) {
    return "historical";
  }
  return classifyError(value);
}

export {
  KernelError,
  OperationConflictError,
  PreDispatchError,
  StaleCapabilityError,
  StoreNotFoundError,
  TransportError,
};
export type { FailureClass };
