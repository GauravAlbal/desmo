import assert from "node:assert/strict";
import { test } from "node:test";
import { LossyIntegerError, parseWire, stringifyWire } from "./json.js";

test("round-trips integers beyond MAX_SAFE_INTEGER as bigint", () => {
  const n = 9007199254740993n;
  const text = stringifyWire({ generation: n });
  assert.equal(text, '{"generation":9007199254740993}');
  const parsed = parseWire(text) as { generation: bigint };
  assert.equal(parsed.generation, n);
});

test("rejects already-lossy JS numbers on stringify", () => {
  const lossy = Number("9007199254740993");
  assert.equal(lossy, 9007199254740992);
  assert.throws(() => stringifyWire({ generation: lossy }), LossyIntegerError);
});

test("keeps safe integers as numbers", () => {
  const parsed = parseWire('{"generation":7}') as { generation: number };
  assert.equal(parsed.generation, 7);
  assert.equal(typeof parsed.generation, "number");
});

test("does not treat sentinel-looking strings as bigints", () => {
  const parsed = parseWire('{"label":"__ik_bigint_0__"}') as { label: string };
  assert.equal(parsed.label, "__ik_bigint_0__");
});

test("does not rewrite integers inside strings", () => {
  const parsed = parseWire('{"message":"ticket 9007199254740993, retry later"}') as {
    message: string;
  };
  assert.equal(parsed.message, "ticket 9007199254740993, retry later");
});

test("preserves long fractional decimals as numbers", () => {
  const parsed = parseWire('{"value":0.9999999999999999}') as { value: number };
  assert.equal(typeof parsed.value, "number");
  assert.equal(parsed.value, Number("0.9999999999999999"));
});

test("does not coerce ordinary __ik_bigint objects on stringify", () => {
  assert.equal(
    stringifyWire({ payload: { __ik_bigint: "42" } }),
    '{"payload":{"__ik_bigint":"42"}}',
  );
});

test("round-trips nested structures, escapes, and integer boundaries", () => {
  const value = {
    nested: [{ ok: true, n: null, s: 'a"b\\c' }],
    hi: 9007199254740991,
    over: 9007199254740993n,
  };
  const text = stringifyWire(value);
  const parsed = parseWire(text) as typeof value;
  assert.equal(parsed.hi, 9007199254740991);
  assert.equal(parsed.over, 9007199254740993n);
  assert.equal(parsed.nested[0].s, 'a"b\\c');
});
