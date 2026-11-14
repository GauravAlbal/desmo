/**
 * Generate TypeScript command/state/outcome contracts from `ik schema` output.
 *
 * The generator never parses `.ik.toml` and never evaluates guards: Rust owns the machine
 * semantics and exports shapes only. Output is deterministic and disposable; `desmo codegen
 * --check` fails when it no longer matches the definition.
 */
import { isRecord } from "./guards.js";

export const SCHEMA_ID = "ikaros.machine-schema.v1";

export type SchemaType = "int" | "string" | "bool";

export interface MachineSchema {
  machine: { machine_id: string; version: string; spec_hash: string };
  states: { name: string; initial: boolean; terminal: boolean }[];
  facts: { name: string; type: SchemaType; policy: string; required: boolean }[];
  commands: {
    name: string;
    from: string[];
    args: { name: string; type: SchemaType }[];
    successors: string[];
    conditional_successor: boolean;
    plans_effect: boolean;
  }[];
  invariants: string[];
}

export class SchemaError extends Error {
  constructor(message: string) {
    super(`invalid machine schema: ${message}`);
    this.name = "SchemaError";
  }
}

function text(value: unknown, what: string): string {
  if (typeof value !== "string") {
    throw new SchemaError(`${what} must be a string`);
  }
  return value;
}

function flag(value: unknown, what: string): boolean {
  if (typeof value !== "boolean") {
    throw new SchemaError(`${what} must be a boolean`);
  }
  return value;
}

function schemaType(value: unknown, what: string): SchemaType {
  if (value === "int" || value === "string" || value === "bool") {
    return value;
  }
  throw new SchemaError(`${what} has unsupported type ${JSON.stringify(value)}`);
}

function rows(value: unknown, what: string): Record<string, unknown>[] {
  if (!Array.isArray(value)) {
    throw new SchemaError(`${what} must be an array`);
  }
  return value.map((row) => {
    if (!isRecord(row)) {
      throw new SchemaError(`${what} entries must be objects`);
    }
    return row;
  });
}

function names(value: unknown, what: string): string[] {
  if (!Array.isArray(value)) {
    throw new SchemaError(`${what} must be an array`);
  }
  return value.map((item) => text(item, what));
}

/** Validate untrusted `ik schema` JSON. */
export function parseMachineSchema(value: unknown): MachineSchema {
  if (!isRecord(value) || value.schema !== SCHEMA_ID) {
    throw new SchemaError(`schema must be ${SCHEMA_ID}`);
  }
  const machine = value.machine;
  if (!isRecord(machine)) {
    throw new SchemaError("machine identity is required");
  }
  return {
    machine: {
      machine_id: text(machine.machine_id, "machine_id"),
      version: String(machine.version),
      spec_hash: text(machine.spec_hash, "spec_hash"),
    },
    states: rows(value.states, "states").map((row) => ({
      name: text(row.name, "state name"),
      initial: flag(row.initial, "state.initial"),
      terminal: flag(row.terminal, "state.terminal"),
    })),
    facts: rows(value.facts, "facts").map((row) => ({
      name: text(row.name, "fact name"),
      type: schemaType(row.type, "fact"),
      policy: text(row.policy, "fact policy"),
      required: flag(row.required, "fact.required"),
    })),
    commands: rows(value.commands, "commands").map((row) => ({
      name: text(row.name, "command name"),
      from: names(row.from, "command.from"),
      args: rows(row.args, "command.args").map((arg) => ({
        name: text(arg.name, "arg name"),
        type: schemaType(arg.type, "arg"),
      })),
      successors: names(row.successors, "command.successors"),
      conditional_successor: flag(row.conditional_successor, "conditional_successor"),
      plans_effect: flag(row.plans_effect, "plans_effect"),
    })),
    invariants: names(value.invariants, "invariants"),
  };
}

const TS_TYPES: Record<SchemaType, string> = {
  int: "number | bigint",
  string: "string",
  bool: "boolean",
};

function lit(value: string): string {
  return JSON.stringify(value);
}

function union(values: string[]): string {
  return values.length === 0 ? "never" : values.map(lit).join(" | ");
}

function tuple(values: string[]): string {
  return `[${values.map(lit).join(", ")}]`;
}

function argsType(args: MachineSchema["commands"][number]["args"]): string {
  if (args.length === 0) {
    return "Record<string, never>";
  }
  return `{ ${args.map((arg) => `${lit(arg.name)}: ${TS_TYPES[arg.type]}`).join("; ")} }`;
}

function commandUnion(schema: MachineSchema): string {
  if (schema.commands.length === 0) {
    return "never";
  }
  return schema.commands
    .map((cmd) =>
      cmd.args.length === 0
        ? `  | { name: ${lit(cmd.name)}; args?: never }`
        : `  | { name: ${lit(cmd.name)}; args: CommandArgs[${lit(cmd.name)}] }`,
    )
    .join("\n");
}

/** Produce the generated module text. Pure and deterministic. */
export function generateContracts(schema: MachineSchema, options: { importFrom: string }): string {
  const { machine } = schema;
  const terminal = schema.states.filter((s) => s.terminal).map((s) => s.name);
  const facts = schema.facts
    .map((f) => `  readonly ${lit(f.name)}${f.required ? "" : "?"}: ${TS_TYPES[f.type]};`)
    .join("\n");
  const argsBlock = schema.commands
    .map((cmd) => `  ${lit(cmd.name)}: ${argsType(cmd.args)};`)
    .join("\n");
  const sources = schema.commands
    .map((cmd) => `  ${lit(cmd.name)}: ${tuple(cmd.from)},`)
    .join("\n");
  const successors = schema.commands
    .map((cmd) => `  ${lit(cmd.name)}: ${union(cmd.successors)};`)
    .join("\n");
  return `// GENERATED by \`desmo codegen\` from machine ${machine.machine_id} v${machine.version}. Do not edit.
//
// A disposable projection of the machine definition: regenerate after any definition change;
// \`desmo codegen --check\` fails when this file is stale. These types are host-language
// documentation, not authority. An aliased or copied handle still type-checks and is refused
// by the kernel at runtime (IK_CAPABILITY_STALE), and a cast cannot bypass invariant or
// currentness checks. A command whose outcome has several successors yields a union; do not
// narrow it without reading the committed state.
//
// Recovery: if \`execute\` rejects with a submission-unknown error, reconcile with
// \`machine.lookupOperation(...)\` using \`error.operationId\`; an exact repeat resolves to a
// HistoricalResult, which carries no capability.
import type { Aggregate, HistoricalResult, Json } from ${lit(options.importFrom)};

export type { HistoricalResult, OperationLookup } from ${lit(options.importFrom)};

export const MACHINE = {
  machine_id: ${lit(machine.machine_id)},
  version: ${lit(machine.version)},
  spec_hash: ${lit(machine.spec_hash)},
} as const;

export const STATES = ${tuple(schema.states.map((s) => s.name))} as const;
export type StateName = (typeof STATES)[number];
export type TerminalState = ${union(terminal)};

/** Facts that may be absent (no initial value) are optional: no total field is invented. */
export interface Facts {
${facts}
}

export type CommandArgs = {
${argsBlock}
};

export type Command =
${commandUnion(schema)};
export type CommandName = Command["name"];

/** States each command is defined from. */
export const COMMAND_SOURCES = {
${sources}
} as const;

/** Possible successor states per command; several successors stay a union. */
export type Successors = {
${successors}
};
export type SuccessorOf<N extends CommandName> = Successors[N];

export class StaleContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StaleContractError";
  }
}

/** Throw when the bound machine is not the one this file was generated from. */
export function assertMachine(machine: {
  identity: { machine_id: string; version: number | bigint | string; spec_hash: string };
}): void {
  const { identity } = machine;
  if (
    identity.machine_id !== MACHINE.machine_id ||
    String(identity.version) !== MACHINE.version ||
    identity.spec_hash !== MACHINE.spec_hash
  ) {
    throw new StaleContractError(
      \`generated contract is for \${MACHINE.machine_id} v\${MACHINE.version} (\${MACHINE.spec_hash}); \` +
        \`bound machine is \${identity.machine_id} v\${String(identity.version)} (\${identity.spec_hash}). Regenerate with \\\`desmo codegen\\\`.\`,
    );
  }
}

/** Compile-time shape check; returns its argument unchanged. */
export function command<C extends Command>(value: C): C {
  return value;
}

/** Type-checked \`aggregate.execute\` that first verifies the generated identity. */
export async function execute(
  aggregate: Aggregate,
  value: Command,
  options: { operationId?: string } = {},
): Promise<Aggregate | HistoricalResult> {
  assertMachine(aggregate.machine);
  const wire: Record<string, Json> =
    value.args === undefined ? { name: value.name } : { name: value.name, args: value.args };
  return aggregate.execute(wire, options);
}
`;
}
