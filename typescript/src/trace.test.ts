import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { TraceValidationError, parseTraceBundle, renderTraceHtml } from "./trace.js";

const cliJs = join(fileURLToPath(new URL("..", import.meta.url)), "dist", "cli.js");

function bundle(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schema: "ikaros.trace.v2",
    product: "Ikaros",
    runtime_version: "0.5.0",
    advisory: true,
    origin: "observed",
    reproducible: false,
    redacted: false,
    events: [],
    ...overrides,
  };
}

test("a read-only show is not rendered as a commit and cannot be mislabelled as one", () => {
  const html = renderTraceHtml(
    bundle({
      events: [{ kind: "read", attempted: "show", state: "cancelled", generation: "2" }],
    }),
  );
  assert.match(html, /read-only observation \(not a mutation\)/);
  assert.equal(html.includes("committed mutation"), false);

  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          events: [{ kind: "commit", attempted: "show", state: "cancelled", generation: "2" }],
        }),
      ),
    TraceValidationError,
  );
  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          events: [
            { kind: "read", attempted: "show", state: "a", generation: "1", toState: "b" },
          ],
        }),
      ),
    /must not carry "toState"/,
  );
});

test("contradictory event fields are rejected rather than rendered", () => {
  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          events: [
            {
              kind: "refusal",
              attempted: "submit",
              fromState: "cancelled",
              code: "IK_CAPABILITY_STALE",
              message: "stale",
              toState: "submitted",
            },
          ],
        }),
      ),
    /must not carry "toState"/,
  );
  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          events: [
            {
              kind: "commit",
              attempted: "start",
              fromState: "created",
              toState: "running",
              generation: "1",
              code: "IK_CAPABILITY_STALE",
            },
          ],
        }),
      ),
    /must not carry "code"/,
  );
  assert.throws(
    () => parseTraceBundle(bundle({ events: [{ kind: "teleport", attempted: "x" }] })),
    /no recognised kind/,
  );
});

test("a refused destination is stated as never reached, and unknown stays unknown", () => {
  const html = renderTraceHtml(
    bundle({
      events: [
        {
          kind: "refusal",
          attempted: "submit",
          fromState: "cancelled",
          intendedState: "submitted",
          code: "IK_CAPABILITY_STALE",
          message: "stale",
        },
        {
          kind: "unknown",
          attempted: "start",
          message: "timed out",
          continuation: "look the operation up",
        },
      ],
    }),
  );
  assert.match(html, /stayed at cancelled; never reached submitted/);
  assert.match(html, /outcome unknown \(submission may have committed\)/);
  assert.equal(html.includes("committed mutation"), false);
});

test("an exact-duplicate response is replayed history, not a new commit", () => {
  const html = renderTraceHtml(
    bundle({
      events: [
        {
          kind: "historical",
          attempted: "start",
          operationId: "op-1",
          firstSequence: "1",
          lastSequence: "1",
        },
      ],
    }),
  );
  assert.match(html, /replayed history of an earlier commit \(not a new commit\)/);
  assert.equal(html.includes("committed mutation"), false);
});

test("a redacted trace cannot claim to be a reproducible original run", () => {
  assert.throws(
    () => parseTraceBundle(bundle({ redacted: true, reproducible: true, reproduction: "cmd" })),
    /redacted trace cannot claim/,
  );
  assert.throws(
    () => parseTraceBundle(bundle({ redacted: true, reproduction: "cmd" })),
    /redacted trace cannot claim/,
  );
  const html = renderTraceHtml(bundle({ redacted: true }));
  assert.match(html, /not a reproducible original run/);
});

test("replayed and witness exports identify their own provenance", () => {
  assert.throws(() => parseTraceBundle(bundle({ origin: "replayed" })), /must identify its inputs/);
  assert.throws(
    () => parseTraceBundle(bundle({ origin: "replayed", inputs: "log", reproducible: true, reproduction: "c" })),
    /not an original run/,
  );
  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          origin: "bounded-witness",
          events: [{ kind: "commit", attempted: "a", fromState: "x", toState: "y", generation: "1" }],
        }),
      ),
    /impossible in a bounded-witness trace/,
  );
  assert.throws(
    () =>
      parseTraceBundle(
        bundle({
          events: [{ kind: "witness", attempted: "a", fromState: "x", toState: "y" }],
        }),
      ),
    /impossible in a observed trace/,
  );
});

test("one observed run and one bounded witness render with different labels", () => {
  const observed = renderTraceHtml(
    bundle({
      events: [{ kind: "commit", attempted: "start", fromState: "created", toState: "running", generation: "1" }],
    }),
  );
  const witness = renderTraceHtml(
    bundle({
      origin: "bounded-witness",
      events: [{ kind: "witness", attempted: "start", fromState: "created", toState: "running" }],
    }),
  );
  assert.match(observed, /committed mutation/);
  assert.equal(observed.includes("bounded witness"), false);
  assert.match(witness, /bounded witness \(checker evidence, not an observed run\)/);
  assert.match(witness, /nothing here was executed against a store/);
  assert.equal(witness.includes("committed mutation"), false);
});

test("hostile labels are escaped in every free-text field", () => {
  const hostile = '<img src=x onerror="alert(1)">';
  const html = renderTraceHtml(
    bundle({
      machine_id: hostile,
      events: [
        {
          kind: "refusal",
          attempted: hostile,
          fromState: hostile,
          intendedState: hostile,
          code: hostile,
          message: hostile,
          continuation: hostile,
        },
      ],
    }),
  );
  assert.equal(html.includes("<img"), false);
  assert.equal(html.includes("&lt;img"), true);
});

test("`desmo trace` renders a valid bundle and refuses a contradictory one without writing output", () => {
  const dir = mkdtempSync(join(tmpdir(), "desmo-trace-"));
  try {
    const good = join(dir, "good.json");
    const bad = join(dir, "bad.json");
    writeFileSync(good, JSON.stringify(bundle({ events: [{ kind: "read", attempted: "show", state: "draft", generation: "0" }] })));
    writeFileSync(bad, JSON.stringify(bundle({ events: [{ kind: "witness", attempted: "x", fromState: "a", toState: "b" }] })));
    const ok = spawnSync(process.execPath, [cliJs, "trace", good, "--out", join(dir, "good.html")], { encoding: "utf8" });
    assert.equal(ok.status, 0);
    assert.match(readFileSync(join(dir, "good.html"), "utf8"), /read-only observation \(not a mutation\)/);
    const refused = spawnSync(process.execPath, [cliJs, "trace", bad, "--out", join(dir, "bad.html")], { encoding: "utf8" });
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /impossible in a observed trace/);
    assert.throws(() => readFileSync(join(dir, "bad.html")), /ENOENT/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
