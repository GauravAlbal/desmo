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
export const RUNTIME_VERSION = "0.5.0";
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

export class Kernel {
  readonly binary: string;
  readonly store: string;
  readonly timeoutMs: number;
  runtimeVersion = RUNTIME_VERSION;
  protocol = { major: PROTOCOL_MAJOR, minor: PROTOCOL_MINOR };
  private nextId = 1;

  constructor(store: string, options: { binary?: string; timeoutMs?: number } = {}) {
    this.store = store;
    this.binary = locateBinary(options.binary);
    this.timeoutMs = options.timeoutMs ?? 30_000;
  }

  /** A fresh caller operation id. Persist it before dispatch to survive your own crash. */
  newOperationId(): string {
    return `ts-${randomUUID()}`;
  }

  machine(path: string): Promise<Machine> {
    return this.loadMachine(path);
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
    const argv = [this.binary, "--store", this.store];
    if (machine) {
      argv.push("--machine", machine.path);
    }
    argv.push(op);
    const envelope = await runJson(argv[0], argv.slice(1), this.timeoutMs, stringifyWire(env));
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
  const timer = setTimeout(() => {
    child.kill("SIGKILL");
    reject(new TransportError(`ik timed out after ${timeoutMs}ms`));
  }, timeoutMs);
  child.stdout.on("data", (c: Buffer) => chunks.push(c));
  child.stderr.on("data", (c: Buffer) => errChunks.push(c));
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
