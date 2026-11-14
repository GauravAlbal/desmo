import { isRecord } from "./guards.js";

/**
 * `ikaros.trace.v2`: an advisory, read-only rendering input.
 *
 * Every event is a tagged variant so impossible combinations (a read that is also a commit, a
 * refusal that reaches its destination) cannot be expressed, and `parseTraceBundle` rejects
 * contradictory fields instead of rendering them. A trace never grants authority: the kernel does.
 */
export type TraceEvent =
  | { kind: "commit"; attempted: string; fromState: string; toState: string; generation: string }
  | {
      kind: "refusal";
      attempted: string;
      fromState: string;
      intendedState?: string;
      generation?: string;
      code: string;
      message: string;
      continuation?: string;
    }
  | { kind: "no-op"; attempted: string; state: string; generation?: string; message?: string }
  | {
      kind: "unknown";
      attempted: string;
      fromState?: string;
      message: string;
      continuation: string;
    }
  | { kind: "read"; attempted: string; state: string; generation: string; message?: string }
  | {
      kind: "historical";
      attempted: string;
      operationId: string;
      firstSequence: string;
      lastSequence: string;
      message?: string;
    }
  | { kind: "witness"; attempted: string; fromState: string; toState: string; message?: string };

export type TraceKind = TraceEvent["kind"];
export type TraceOrigin = "observed" | "replayed" | "bounded-witness";

export type TraceBundle = {
  schema: "ikaros.trace.v2";
  product: "Ikaros";
  runtime_version: string;
  machine_id?: string;
  advisory: true;
  /** `observed`: a run that happened. `replayed`: re-presented from `inputs`. `bounded-witness`: a checker witness, not a run. */
  origin: TraceOrigin;
  /** True only when `reproduction` can regenerate this exact trace. */
  reproducible: boolean;
  /** True when content was removed; a redacted trace is never reproducible. */
  redacted: boolean;
  /** Required for `replayed`: what this trace was rebuilt from. */
  inputs?: string;
  events: TraceEvent[];
  reproduction?: string;
};

const EVENT_FIELDS: Record<TraceKind, { required: string[]; optional: string[] }> = {
  commit: { required: ["attempted", "fromState", "toState", "generation"], optional: [] },
  refusal: {
    required: ["attempted", "fromState", "code", "message"],
    optional: ["intendedState", "generation", "continuation"],
  },
  "no-op": { required: ["attempted", "state"], optional: ["generation", "message"] },
  unknown: { required: ["attempted", "message", "continuation"], optional: ["fromState"] },
  read: { required: ["attempted", "state", "generation"], optional: ["message"] },
  historical: {
    required: ["attempted", "operationId", "firstSequence", "lastSequence"],
    optional: ["message"],
  },
  witness: { required: ["attempted", "fromState", "toState"], optional: ["message"] },
};

const ALLOWED_KINDS: Record<TraceOrigin, TraceKind[]> = {
  observed: ["commit", "refusal", "no-op", "unknown", "read", "historical"],
  replayed: ["commit", "refusal", "no-op", "unknown", "read", "historical"],
  "bounded-witness": ["witness"],
};

export class TraceValidationError extends Error {
  constructor(message: string) {
    super(`invalid trace: ${message}`);
    this.name = "TraceValidationError";
  }
}

function isTraceKind(value: unknown): value is TraceKind {
  return typeof value === "string" && Object.hasOwn(EVENT_FIELDS, value);
}

function isTraceOrigin(value: unknown): value is TraceOrigin {
  return value === "observed" || value === "replayed" || value === "bounded-witness";
}

function checkEvent(raw: unknown, index: number, origin: TraceOrigin): TraceEvent {
  if (!isRecord(raw) || !isTraceKind(raw.kind)) {
    throw new TraceValidationError(`event ${index} has no recognised kind`);
  }
  const kind = raw.kind;
  if (!ALLOWED_KINDS[origin].includes(kind)) {
    throw new TraceValidationError(`event ${index} kind "${kind}" is impossible in a ${origin} trace`);
  }
  const { required, optional } = EVENT_FIELDS[kind];
  for (const field of required) {
    if (typeof raw[field] !== "string") {
      throw new TraceValidationError(`event ${index} (${kind}) requires string field "${field}"`);
    }
  }
  for (const [field, value] of Object.entries(raw)) {
    if (field === "kind" || required.includes(field)) {
      continue;
    }
    if (!optional.includes(field)) {
      throw new TraceValidationError(`event ${index} (${kind}) must not carry "${field}"`);
    }
    if (typeof value !== "string") {
      throw new TraceValidationError(`event ${index} (${kind}) field "${field}" must be a string`);
    }
  }
  return raw as TraceEvent;
}

/** Validate untrusted JSON into a TraceBundle, rejecting every contradictory combination. */
export function parseTraceBundle(value: unknown): TraceBundle {
  if (!isRecord(value)) {
    throw new TraceValidationError("bundle is not an object");
  }
  if (value.schema !== "ikaros.trace.v2") {
    throw new TraceValidationError(`unsupported schema ${String(value.schema)}`);
  }
  if (value.advisory !== true || value.product !== "Ikaros") {
    throw new TraceValidationError("a trace must declare itself advisory");
  }
  if (typeof value.runtime_version !== "string") {
    throw new TraceValidationError("runtime_version is required");
  }
  if (!isTraceOrigin(value.origin)) {
    throw new TraceValidationError("origin must be observed, replayed, or bounded-witness");
  }
  if (typeof value.reproducible !== "boolean" || typeof value.redacted !== "boolean") {
    throw new TraceValidationError("reproducible and redacted are required booleans");
  }
  if (value.redacted && (value.reproducible || value.reproduction !== undefined)) {
    throw new TraceValidationError("a redacted trace cannot claim to be a reproducible original run");
  }
  if (value.reproducible && typeof value.reproduction !== "string") {
    throw new TraceValidationError("a reproducible trace must name its reproduction command");
  }
  if (value.origin !== "observed" && value.reproducible) {
    throw new TraceValidationError(`a ${value.origin} trace is not an original run`);
  }
  if (value.origin === "replayed" && typeof value.inputs !== "string") {
    throw new TraceValidationError("a replayed trace must identify its inputs");
  }
  if (!Array.isArray(value.events)) {
    throw new TraceValidationError("events must be an array");
  }
  const origin = value.origin;
  const events = value.events.map((event, index) => checkEvent(event, index, origin));
  return {
    schema: "ikaros.trace.v2",
    product: "Ikaros",
    runtime_version: value.runtime_version,
    machine_id: typeof value.machine_id === "string" ? value.machine_id : undefined,
    advisory: true,
    origin,
    reproducible: value.reproducible,
    redacted: value.redacted,
    inputs: typeof value.inputs === "string" ? value.inputs : undefined,
    events,
    reproduction: typeof value.reproduction === "string" ? value.reproduction : undefined,
  };
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function describe(event: TraceEvent): { label: string; line: string } {
  const attempted = `<strong>${escapeHtml(event.attempted)}</strong>`;
  switch (event.kind) {
    case "commit":
      return {
        label: "committed mutation",
        line: `${attempted} ${escapeHtml(event.fromState)} → ${escapeHtml(event.toState)} (generation ${escapeHtml(event.generation)})`,
      };
    case "refusal":
      return {
        label: `refused ${escapeHtml(event.code)}`,
        line: `${attempted} stayed at ${escapeHtml(event.fromState)}${
          event.intendedState ? `; never reached ${escapeHtml(event.intendedState)}` : ""
        }`,
      };
    case "no-op":
      return { label: "no-op (nothing committed)", line: `${attempted} at ${escapeHtml(event.state)}` };
    case "unknown":
      return {
        label: "outcome unknown (submission may have committed)",
        line: `${attempted}${event.fromState ? ` from ${escapeHtml(event.fromState)}` : ""}`,
      };
    case "read":
      return {
        label: "read-only observation (not a mutation)",
        line: `${attempted} observed ${escapeHtml(event.state)} at generation ${escapeHtml(event.generation)}`,
      };
    case "historical":
      return {
        label: "replayed history of an earlier commit (not a new commit)",
        line: `${attempted} operation ${escapeHtml(event.operationId)}, sequences ${escapeHtml(event.firstSequence)}..${escapeHtml(event.lastSequence)}`,
      };
    case "witness":
      return {
        label: "bounded witness (checker evidence, not an observed run)",
        line: `${attempted} ${escapeHtml(event.fromState)} → ${escapeHtml(event.toState)}`,
      };
  }
}

function provenanceNote(bundle: TraceBundle): string {
  if (bundle.redacted) {
    return "Redacted: content was removed. This is not a reproducible original run.";
  }
  if (bundle.origin === "replayed") {
    return `Replayed from: ${escapeHtml(bundle.inputs ?? "")}. Not an original run.`;
  }
  if (bundle.origin === "bounded-witness") {
    return "Bounded witness from a checker. It describes a possible path within a finite corpus; nothing here was executed against a store.";
  }
  return bundle.reproducible ? "Observed run with a reproduction command." : "Observed run; no reproduction command.";
}

export function renderTraceHtml(input: unknown): string {
  const bundle = parseTraceBundle(input);
  const rows = bundle.events
    .map((event) => {
      const { label, line } = describe(event);
      const message = "message" in event && event.message ? `<pre>${escapeHtml(event.message)}</pre>` : "";
      const next = "continuation" in event && event.continuation ? `<p>${escapeHtml(event.continuation)}</p>` : "";
      return `<li class="${event.kind}">${line}<div class="meta">${label}</div>${message}${next}</li>`;
    })
    .join("");
  return `<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"/><title>Desmo trace</title>
<style>
body{font:16px/1.4 system-ui;margin:2rem;background:#111;color:#eee}
li{padding:.5rem 1rem;margin:1rem 0;border-left:4px solid #888}
.commit{border-color:#4a4}.refusal{border-color:#c44}.unknown{border-color:#ca4}
.read{border-color:#48c}.historical{border-color:#a6c}.witness{border-color:#999;border-style:dashed}
.no-op{border-color:#777}
.meta{opacity:.7;font-size:.9rem}
.note{font-size:.85rem;opacity:.6}
</style></head><body>
<p class="note">Advisory visualization. The kernel, not this page, is authorization. A refused command never reaches its destination state.</p>
<p class="note">${provenanceNote(bundle)}</p>
<p>runtime ${escapeHtml(bundle.runtime_version)}${bundle.machine_id ? " · " + escapeHtml(bundle.machine_id) : ""} · ${escapeHtml(bundle.origin)}</p>
<ol>${rows}</ol>
${bundle.reproduction ? `<p class="note">Reproduce: <code>${escapeHtml(bundle.reproduction)}</code></p>` : ""}
</body></html>`;
}
