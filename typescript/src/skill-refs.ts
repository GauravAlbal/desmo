export type Invocation = {
  raw: string;
  binary: "ik" | "desmo";
  words: string[];
  flags: string[];
  source: string;
};

const COMMAND = /`([^`\n]*\b(?:ik|desmo)\b[^`\n]*)`/g;
const REPO_PATH = /`((?:docs|machines|examples|skills|typescript)\/[A-Za-z0-9_./-]+)`/g;

export function extractInvocations(text: string, source: string): Invocation[] {
  const raws = new Set<string>();
  for (const match of text.matchAll(COMMAND)) {
    const raw = match[1].trim();
    if (/^(?:ik|desmo)(?:\s|$)/.test(raw)) {
      raws.add(raw);
    }
  }
  let fence = false;
  for (const line of text.split("\n")) {
    if (line.startsWith("```")) {
      fence = !fence;
      continue;
    }
    const trimmed = line.trim();
    if (fence && /^(?:ik|desmo)\b/.test(trimmed)) {
      raws.add(trimmed);
    }
  }
  return [...raws].map((raw) => parseInvocation(raw, source));
}

export function extractRepoPaths(text: string): string[] {
  return [...new Set([...text.matchAll(REPO_PATH)].map((match) => match[1]))];
}

export function acceptedFlags(usage: string): Set<string> {
  const flags = new Set<string>();
  for (const line of usage.split("\n")) {
    for (const match of line.matchAll(/--[a-z0-9-]+/g)) {
      const before = line.slice(0, match.index ?? 0);
      if (/\bno\s+$/.test(before)) {
        continue;
      }
      flags.add(match[0]);
    }
  }
  return flags;
}

export function referenceProblems(invocation: Invocation, usage: string): string[] {
  const flags = acceptedFlags(usage);
  const problems: string[] = [];
  for (const flag of invocation.flags) {
    if (!flags.has(flag)) {
      problems.push(`${invocation.source}: flag ${flag} is not in ${invocation.binary} usage (${invocation.raw})`);
    }
  }
  for (const word of invocation.words) {
    if (!usage.includes(word)) {
      problems.push(`${invocation.source}: subcommand ${word} is not in ${invocation.binary} usage (${invocation.raw})`);
    }
  }
  return problems;
}

function parseInvocation(raw: string, source: string): Invocation {
  const binary = raw.startsWith("desmo") ? "desmo" : "ik";
  const tokens = tokenize(raw);
  const flags = tokens.filter((token) => token.startsWith("--"));
  const words = tokens.filter((token) => token !== binary && !token.startsWith("--"));
  return { raw, binary, words, flags, source };
}

function tokenize(command: string): string[] {
  return command
    .replace(/<[^>]*>/g, " ")
    .replace(/[\[\]]/g, " ")
    .split(/\s+/)
    .filter((token) => token.length > 0 && token !== "..." && !token.startsWith("<"));
}
