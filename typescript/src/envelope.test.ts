import assert from "node:assert/strict";
import { test } from "node:test";
import { unwrapEnvelope } from "./envelope.js";
import { ProtocolError, TransportError } from "./errors.js";
import { PROTOCOL_MAJOR, PROTOCOL_MINOR, RUNTIME_VERSION } from "./client.js";

const expected = {
  request_id: "ts-1",
  runtime_version: RUNTIME_VERSION,
  protocol: { major: PROTOCOL_MAJOR, minor: PROTOCOL_MINOR },
};

test("rejects mismatched request_id", () => {
  assert.throws(
    () =>
      unwrapEnvelope(
        {
          protocol: { major: 1, minor: 0 },
          runtime_version: RUNTIME_VERSION,
          request_id: "other",
          ok: true,
          result: { aggregate_id: "x" },
        },
        expected,
      ),
    TransportError,
  );
});

test("rejects mismatched runtime_version", () => {
  assert.throws(
    () =>
      unwrapEnvelope(
        {
          protocol: { major: 1, minor: 0 },
          runtime_version: "0.0.0",
          request_id: "ts-1",
          ok: true,
          result: { aggregate_id: "x" },
        },
        expected,
      ),
    TransportError,
  );
});

test("rejects mismatched protocol", () => {
  assert.throws(
    () =>
      unwrapEnvelope(
        {
          protocol: { major: 9, minor: 0 },
          runtime_version: RUNTIME_VERSION,
          request_id: "ts-1",
          ok: true,
          result: { aggregate_id: "x" },
        },
        expected,
      ),
    TransportError,
  );
});

test("empty request id with a diagnostic is a protocol refusal", () => {
  assert.throws(
    () =>
      unwrapEnvelope(
        {
          protocol: { major: 1, minor: 0 },
          runtime_version: RUNTIME_VERSION,
          request_id: "",
          ok: false,
          diagnostic: { code: "IK_PROTOCOL_INVALID", message: "malformed request" },
        },
        expected,
      ),
    (err: unknown) => err instanceof ProtocolError && err.code === "IK_PROTOCOL_INVALID",
  );
});
