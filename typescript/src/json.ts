/** Wire JSON that preserves integers beyond Number.MAX_SAFE_INTEGER. */

export type Json = null | boolean | number | bigint | string | Json[] | { [k: string]: Json };

const MAX = BigInt(Number.MAX_SAFE_INTEGER);
const MIN = BigInt(Number.MIN_SAFE_INTEGER);

export class LossyIntegerError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "LossyIntegerError";
  }
}

export function assertSafeWireValue(value: unknown, path = "$"): void {
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new LossyIntegerError(`${path}: non-finite number is not legal wire JSON`);
    }
    if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
      throw new LossyIntegerError(
        `${path}: integer ${value} is outside Number.MAX_SAFE_INTEGER; pass bigint`,
      );
    }
    return;
  }
  if (typeof value === "bigint") {
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, i) => assertSafeWireValue(item, `${path}[${i}]`));
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, nested] of Object.entries(value as Record<string, unknown>)) {
      assertSafeWireValue(nested, `${path}.${key}`);
    }
  }
}

export function stringifyWire(value: unknown): string {
  assertSafeWireValue(value);
  return writeJson(value);
}

function writeJson(value: unknown): string {
  if (value === null) {
    return "null";
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "bigint") {
    return value.toString();
  }
  if (typeof value === "number") {
    return JSON.stringify(value);
  }
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((item) => writeJson(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).map(
      ([key, nested]) => `${JSON.stringify(key)}:${writeJson(nested)}`,
    );
    return `{${entries.join(",")}}`;
  }
  throw new LossyIntegerError(`cannot serialize ${typeof value}`);
}

class Parser {
  i = 0;
  constructor(readonly text: string) {}

  parseValue(): unknown {
    this.skipWs();
    const ch = this.text[this.i];
    if (ch === "{") {
      return this.parseObject();
    }
    if (ch === "[") {
      return this.parseArray();
    }
    if (ch === '"') {
      return this.parseString();
    }
    if (ch === "t" || ch === "f") {
      return this.parseLiteral();
    }
    if (ch === "n") {
      this.expect("null");
      return null;
    }
    if (ch === "-" || (ch !== undefined && ch >= "0" && ch <= "9")) {
      return this.parseNumber();
    }
    throw new SyntaxError(`unexpected ${ch ?? "end"} at ${this.i}`);
  }

  parseObject(): Record<string, unknown> {
    this.expect("{");
    this.skipWs();
    const obj: Record<string, unknown> = {};
    if (this.text[this.i] === "}") {
      this.i += 1;
      return obj;
    }
    while (true) {
      this.skipWs();
      const key = this.parseString();
      this.skipWs();
      this.expect(":");
      obj[key] = this.parseValue();
      this.skipWs();
      if (this.text[this.i] === ",") {
        this.i += 1;
        continue;
      }
      this.expect("}");
      return obj;
    }
  }

  parseArray(): unknown[] {
    this.expect("[");
    this.skipWs();
    const arr: unknown[] = [];
    if (this.text[this.i] === "]") {
      this.i += 1;
      return arr;
    }
    while (true) {
      arr.push(this.parseValue());
      this.skipWs();
      if (this.text[this.i] === ",") {
        this.i += 1;
        continue;
      }
      this.expect("]");
      return arr;
    }
  }

  parseString(): string {
    this.expect('"');
    let out = "";
    while (this.i < this.text.length) {
      const ch = this.text[this.i];
      if (ch === '"') {
        this.i += 1;
        return out;
      }
      if (ch === "\\") {
        this.i += 1;
        const esc = this.text[this.i];
        this.i += 1;
        const map: Record<string, string> = {
          '"': '"',
          "\\": "\\",
          "/": "/",
          b: "\b",
          f: "\f",
          n: "\n",
          r: "\r",
          t: "\t",
        };
        if (esc && esc in map) {
          out += map[esc];
          continue;
        }
        if (esc === "u") {
          const hex = this.text.slice(this.i, this.i + 4);
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) {
            throw new SyntaxError("invalid unicode escape");
          }
          out += String.fromCharCode(parseInt(hex, 16));
          this.i += 4;
          continue;
        }
        throw new SyntaxError(`invalid escape ${esc}`);
      }
      this.i += 1;
      out += ch;
    }
    throw new SyntaxError("unterminated string");
  }

  parseNumber(): number | bigint {
    const start = this.i;
    if (this.text[this.i] === "-") {
      this.i += 1;
    }
    if (this.text[this.i] === "0") {
      this.i += 1;
    } else {
      if (this.text[this.i] === undefined || this.text[this.i] < "1" || this.text[this.i] > "9") {
        throw new SyntaxError("invalid number");
      }
      while (this.text[this.i] >= "0" && this.text[this.i] <= "9") {
        this.i += 1;
      }
    }
    let frac = false;
    let exp = false;
    if (this.text[this.i] === ".") {
      frac = true;
      this.i += 1;
      if (this.text[this.i] === undefined || this.text[this.i] < "0" || this.text[this.i] > "9") {
        throw new SyntaxError("invalid fraction");
      }
      while (this.text[this.i] >= "0" && this.text[this.i] <= "9") {
        this.i += 1;
      }
    }
    if (this.text[this.i] === "e" || this.text[this.i] === "E") {
      exp = true;
      this.i += 1;
      if (this.text[this.i] === "+" || this.text[this.i] === "-") {
        this.i += 1;
      }
      if (this.text[this.i] === undefined || this.text[this.i] < "0" || this.text[this.i] > "9") {
        throw new SyntaxError("invalid exponent");
      }
      while (this.text[this.i] >= "0" && this.text[this.i] <= "9") {
        this.i += 1;
      }
    }
    const token = this.text.slice(start, this.i);
    if (!frac && !exp) {
      const n = BigInt(token);
      if (n <= MAX && n >= MIN) {
        return Number(n);
      }
      return n;
    }
    const num = Number(token);
    if (!Number.isFinite(num)) {
      throw new LossyIntegerError(`non-finite number ${token}`);
    }
    return num;
  }

  parseLiteral(): boolean {
    if (this.text.startsWith("true", this.i)) {
      this.i += 4;
      return true;
    }
    this.expect("false");
    return false;
  }

  skipWs(): void {
    while (this.i < this.text.length && " \t\n\r".includes(this.text[this.i])) {
      this.i += 1;
    }
  }

  expect(s: string): void {
    if (!this.text.startsWith(s, this.i)) {
      throw new SyntaxError(`expected ${s} at ${this.i}`);
    }
    this.i += s.length;
  }
}

export function parseWire(text: string): unknown {
  const parser = new Parser(text);
  const value = parser.parseValue();
  parser.skipWs();
  if (parser.i !== text.length) {
    throw new SyntaxError(`trailing data at ${parser.i}`);
  }
  return value;
}
