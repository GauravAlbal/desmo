export {
  Kernel,
  Machine,
  Aggregate,
  MachineIdentity,
  Capability,
  Effect,
  EffectIntent,
  HistoricalResult,
  Invocation,
  OperationLookup,
  PROTOCOL_MAJOR,
  PROTOCOL_MINOR,
  RUNTIME_VERSION,
  classify,
} from "./client.js";
export type { FailureClass } from "./client.js";
export type { Json, OperationBody } from "./client.js";
export {
  CommandInvalidError,
  EffectCompletionError,
  EffectConflictError,
  EffectMismatchError,
  EffectStaleError,
  IllegalStateError,
  InvariantFailedError,
  KernelError,
  MachineError,
  NonceInvalidError,
  OperationConflictError,
  PreDispatchError,
  ProtocolError,
  StaleCapabilityError,
  StoreNotFoundError,
  TransportError,
  continuation,
} from "./errors.js";
export type { OutcomeKind } from "./errors.js";
export { parseWire, stringifyWire, assertSafeWireValue, LossyIntegerError } from "./json.js";
export { unwrapEnvelope } from "./envelope.js";
export { locateBinary } from "./locate.js";
export { renderTraceHtml, parseTraceBundle, TraceValidationError } from "./trace.js";
export type { TraceBundle, TraceEvent, TraceKind, TraceOrigin } from "./trace.js";
export { renderSimulatorHtml } from "./simulate.js";
