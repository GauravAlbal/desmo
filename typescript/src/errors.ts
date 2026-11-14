export type OutcomeKind =
  | "committed"
  | "refused"
  | "no-op"
  | "not-sent"
  | "submission-unknown"
  | "effect-outcome-unknown";

export class KernelError extends Error {
  readonly code: string;
  readonly rule?: string;
  readonly diagnostic: Record<string, unknown>;
  readonly outcome: OutcomeKind;
  /** Caller-correlatable id of the logical invocation, when the call was bound to one. */
  operationId?: string;

  constructor(
    code: string,
    message: string,
    diagnostic: Record<string, unknown> = {},
    outcome: OutcomeKind = "refused",
  ) {
    super(`${code}: ${message}`);
    this.name = "KernelError";
    this.code = code;
    this.rule = typeof diagnostic.rule === "string" ? diagnostic.rule : undefined;
    this.diagnostic = diagnostic;
    this.outcome = outcome;
  }
}

export class TransportError extends KernelError {
  constructor(message: string, outcome: OutcomeKind = "submission-unknown") {
    super("IK_TRANSPORT", message, {}, outcome);
    this.name = "TransportError";
  }
}

/** The request was provably never sent (for example the `ik` binary is missing). */
export class PreDispatchError extends TransportError {
  constructor(message: string) {
    super(message, "not-sent");
    this.name = "PreDispatchError";
  }
}

/** `IK_OPERATION_CONFLICT`: the operation id was already committed with different semantics. */
export class OperationConflictError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "OperationConflictError";
  }
}

export class StaleCapabilityError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "StaleCapabilityError";
  }
}

/** `IK_PROTOCOL_*`: the kernel refused the envelope. Nothing was committed. */
export class ProtocolError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "ProtocolError";
  }
}

export class EffectStaleError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "EffectStaleError";
  }
}

export class EffectMismatchError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "EffectMismatchError";
  }
}

export class EffectConflictError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "EffectConflictError";
  }
}

/**
 * The outcome was not sent. The digest was empty, or the intent is not this
 * aggregate's pending intent. This is not submission-unknown: reconcile by
 * reading state, do not treat it as a lost reply.
 */
export class EffectCompletionError extends KernelError {
  constructor(message: string) {
    super("CLIENT_EFFECT_COMPLETION", message, {}, "not-sent");
    this.name = "EffectCompletionError";
  }
}

/** `IK_STORE_NOT_FOUND`: the store file does not exist. Not corruption. */
export class StoreNotFoundError extends KernelError {
  constructor(code: string, message: string, diagnostic: Record<string, unknown>) {
    super(code, message, diagnostic, "refused");
    this.name = "StoreNotFoundError";
  }
}

const TYPES: Record<string, typeof KernelError> = {
  IK_CAPABILITY_STALE: StaleCapabilityError,
  IK_OPERATION_CONFLICT: OperationConflictError,
  IK_PROTOCOL_INVALID: ProtocolError,
  IK_PROTOCOL_VERSION_UNKNOWN: ProtocolError,
  IK_EFFECT_STALE: EffectStaleError,
  IK_EFFECT_MISMATCH: EffectMismatchError,
  IK_EFFECT_OUTCOME_CONFLICT: EffectConflictError,
  IK_STORE_NOT_FOUND: StoreNotFoundError,
};

/** Stable domain-adapter vocabulary. `historical` is a result, not an error. */
export type FailureClass =
  | "not_sent"
  | "refused"
  | "contract_mismatch"
  | "not_found"
  | "corrupt"
  | "historical"
  | "uncertain";

const CORRUPT_CODES = new Set([
  "IK_STORE_CORRUPT",
  "IK_JOURNAL_CORRUPT",
  "IK_PROJECTION_CORRUPT",
  "IK_REPLAY_MISMATCH",
]);
const CONTRACT_CODES = new Set(["IK_EVENT_SCHEMA_UNKNOWN", "IK_PERSISTED_SCHEMA_UNKNOWN"]);
const NOT_FOUND_RULES = new Set(["store.missing", "aggregate.not_found"]);

/** Classify a caught kernel error. Does not return `historical`; that is a result. */
export function classifyError(error: KernelError): Exclude<FailureClass, "historical"> {
  if (error instanceof PreDispatchError || error.outcome === "not-sent") {
    return "not_sent";
  }
  if (
    error instanceof TransportError ||
    error.outcome === "submission-unknown" ||
    error.outcome === "effect-outcome-unknown"
  ) {
    return "uncertain";
  }
  if (error.code === "IK_STORE_NOT_FOUND" || (error.rule !== undefined && NOT_FOUND_RULES.has(error.rule))) {
    return "not_found";
  }
  if (CORRUPT_CODES.has(error.code)) {
    return "corrupt";
  }
  if (
    CONTRACT_CODES.has(error.code) ||
    error.code.startsWith("IK_MACHINE_") ||
    error.code.startsWith("IK_PROTOCOL_")
  ) {
    return "contract_mismatch";
  }
  return "refused";
}

export function raiseDiagnostic(diagnostic: Record<string, unknown>): never {
  const code = String(diagnostic.code ?? "IK_INTERNAL");
  const message = String(diagnostic.message ?? code);
  const Cls = TYPES[code] ?? KernelError;
  throw new Cls(code, message, diagnostic);
}

export function continuation(error: KernelError): string {
  if (error instanceof StaleCapabilityError) {
    return [
      "Canonical state was not changed.",
      "Next: read current state and reconsider the operation.",
      "Do not automatically resubmit this command with fresh authority.",
    ].join("\n");
  }
  if (error.outcome === "not-sent") {
    return "The request was never sent. Fix the cause and send again with the same operation id.";
  }
  if (error.outcome === "submission-unknown") {
    return [
      "The request may have committed.",
      `Next: machine.lookupOperation(...) with operation id ${error.operationId ?? "(none)"} and the original request body.`,
      "Replay and verify audit the journal; they do not answer whether an operation committed.",
    ].join("\n");
  }
  if (error instanceof OperationConflictError) {
    return "This operation id already committed with different semantics. Use a new id for a new decision.";
  }
  return "Canonical state was not changed. Inspect the diagnostic code before retrying.";
}
