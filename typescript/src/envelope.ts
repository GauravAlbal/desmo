import { raiseDiagnostic, TransportError } from "./errors.js";
import { stringifyWire } from "./json.js";
import type { Json } from "./json.js";

export type ExpectedEnvelope = {
  request_id: string;
  runtime_version: string;
  protocol: { major: number; minor: number };
};

export function unwrapEnvelope(
  envelope: unknown,
  expected: ExpectedEnvelope,
): Record<string, Json> {
  if (!envelope || typeof envelope !== "object" || Array.isArray(envelope)) {
    throw new TransportError("ik returned a non-object envelope");
  }
  const rec = envelope as Record<string, Json>;
  if (rec.request_id !== expected.request_id) {
    if (emptyRequestIdDiagnostic(rec)) {
      raiseDiagnostic(rec.diagnostic as Record<string, unknown>);
    }
    throw new TransportError(
      `response request_id ${String(rec.request_id)} !== ${expected.request_id}`,
    );
  }
  if (rec.runtime_version !== expected.runtime_version) {
    throw new TransportError(
      `response runtime_version ${String(rec.runtime_version)} !== ${expected.runtime_version}`,
    );
  }
  const proto = rec.protocol;
  if (!proto || typeof proto !== "object" || Array.isArray(proto)) {
    throw new TransportError("response missing protocol");
  }
  const p = proto as Record<string, Json>;
  if (p.major !== expected.protocol.major || p.minor !== expected.protocol.minor) {
    throw new TransportError(
      `response protocol ${String(p.major)}.${String(p.minor)} !== ${expected.protocol.major}.${expected.protocol.minor}`,
    );
  }
  if (rec.ok === true) {
    const result = rec.result;
    if (!result || typeof result !== "object" || Array.isArray(result)) {
      throw new TransportError("successful envelope missing object result");
    }
    return result as Record<string, Json>;
  }
  const diagnostic = rec.diagnostic;
  if (diagnostic && typeof diagnostic === "object" && !Array.isArray(diagnostic)) {
    raiseDiagnostic(diagnostic as Record<string, unknown>);
  }
  throw new TransportError(`refused envelope without diagnostic: ${stringifyWire(rec)}`);
}

function emptyRequestIdDiagnostic(rec: Record<string, Json>): boolean {
  const diagnostic = rec.diagnostic;
  return (
    (rec.request_id === "" || rec.request_id === undefined) &&
    rec.ok !== true &&
    !!diagnostic &&
    typeof diagnostic === "object" &&
    !Array.isArray(diagnostic)
  );
}
