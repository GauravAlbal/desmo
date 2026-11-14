import fs from "node:fs/promises";

const text = await fs.readFile(process.argv[2], "utf8");
const results = new Map();
for (const line of text.split(/\r?\n/)) {
  const match = line.match(/^(not )?ok \d+ - (R\d+)\b(.*)$/);
  if (!match) continue;
  const failed = Boolean(match[1]);
  const id = match[2];
  const rest = match[3] || "";
  if (/\bSKIP\b/.test(rest)) continue;
  const prev = results.get(id) || "pass";
  results.set(id, failed || prev === "fail" ? "fail" : "pass");
}
const ids = [...results.keys()].sort((a, b) => Number(a.slice(1)) - Number(b.slice(1)));
const summary = {};
for (const id of ids) summary[id] = results.get(id);
process.stdout.write(`${JSON.stringify(summary)}\n`);
process.exit(Object.values(summary).some((v) => v === "fail") ? 1 : 0);
