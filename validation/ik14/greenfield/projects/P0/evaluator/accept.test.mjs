import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";

const PROJECT_DIR = process.env.PROJECT_DIR;
const STUB_DIR = process.env.STUB_DIR;
const OPTS = { timeout: 30000 };

function digestOf(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function desk(stateDir, serviceUrl, args, crash) {
  return new Promise((resolve) => {
    const env = {
      PATH: process.env.PATH,
      HOME: process.env.HOME || "",
      STATE_DIR: stateDir,
      SERVICE_URL: serviceUrl,
    };
    if (crash) env.CRASH_AT = crash;
    const child = spawn(path.join(PROJECT_DIR, "run"), args, { env });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGKILL"), 10000);
    child.stdout.on("data", (d) => { stdout += d; });
    child.stderr.on("data", (d) => { stderr += d; });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      let body = null;
      if (signal == null) {
        try { body = JSON.parse(stdout); } catch { body = null; }
      }
      resolve({ code, signal, stdout, stderr, body });
    });
  });
}

function startStub() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["server.mjs"], {
      cwd: STUB_DIR,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let buf = "";
    let err = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("stub start timeout " + err));
    }, 5000);
    child.stderr.on("data", (d) => { err += d; });
    child.stdout.on("data", (d) => {
      buf += d.toString();
      const nl = buf.indexOf("\n");
      if (nl >= 0) {
        clearTimeout(timer);
        resolve({ child, info: JSON.parse(buf.slice(0, nl)) });
      }
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });
}

async function session(fn) {
  assert.ok(PROJECT_DIR, "PROJECT_DIR is required");
  assert.ok(STUB_DIR, "STUB_DIR is required");
  const stub = await startStub();
  const stateDir = mkdtempSync(path.join(tmpdir(), "p0-"));
  try {
    await fn({
      stateDir,
      service: stub.info.service_url,
      control: stub.info.control_url,
    });
  } finally {
    stub.child.kill("SIGKILL");
    rmSync(stateDir, { recursive: true, force: true });
  }
}

async function effects(control) {
  const res = await fetch(control + "/effects");
  assert.equal(res.status, 200);
  return await res.json();
}

async function setFault(control, body) {
  const res = await fetch(control + "/fault", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  assert.equal(res.status, 200);
}

async function printerOf(service, id) {
  const res = await fetch(service + "/v1/printers/" + id);
  if (res.status === 404) return null;
  assert.equal(res.status, 200);
  return await res.json();
}

function writeNotice(dir, obj) {
  const file = path.join(dir, "notice.json");
  writeFileSync(file, JSON.stringify(obj));
  return file;
}

test("R1 persistence across restart", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from([0, 255, 10, 65, 0]);
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    assert.equal(prep.body.digest, digestOf(bytes));
    assert.equal(prep.body.state, "queued");
    const again = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(again.code, 0);
    assert.equal(again.body.job_id, prep.body.job_id);
    assert.equal(again.body.digest, prep.body.digest);
    assert.equal(again.body.state, "queued");
    assert.deepEqual(Buffer.from(again.body.label_b64, "base64"), bytes);
  });
});

test("R2 hold and cancel do not print", OPTS, async () => {
  await session(async (ctx) => {
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, Buffer.from("hold-me"));
    const a = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(a.code, 0);
    const held = await desk(ctx.stateDir, ctx.service, ["hold", a.body.job_id]);
    assert.equal(held.code, 0);
    assert.equal(held.body.state, "held");
    const denied = await desk(ctx.stateDir, ctx.service, ["dispatch", a.body.job_id]);
    assert.notEqual(denied.code, 0);
    const stA = await desk(ctx.stateDir, ctx.service, ["status", a.body.job_id]);
    assert.equal(stA.body.state, "held");
    assert.equal(stA.body.dispatch_id, null);
    writeFileSync(file, Buffer.from("cancel-me"));
    const b = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(b.code, 0);
    const cancelled = await desk(ctx.stateDir, ctx.service, ["cancel", b.body.job_id]);
    assert.equal(cancelled.code, 0);
    assert.equal(cancelled.body.state, "cancelled");
    const deniedB = await desk(ctx.stateDir, ctx.service, ["dispatch", b.body.job_id]);
    assert.notEqual(deniedB.code, 0);
    const stB = await desk(ctx.stateDir, ctx.service, ["status", b.body.job_id]);
    assert.equal(stB.body.state, "cancelled");
    const cancelHeld = await desk(ctx.stateDir, ctx.service, ["cancel", a.body.job_id]);
    assert.equal(cancelHeld.code, 0);
    const doneA = await desk(ctx.stateDir, ctx.service, ["status", a.body.job_id]);
    const doneB = await desk(ctx.stateDir, ctx.service, ["status", b.body.job_id]);
    assert.equal(doneA.body.state, "cancelled");
    assert.equal(doneB.body.state, "cancelled");
    assert.equal((await effects(ctx.control)).count, 0);
    assert.equal(await printerOf(ctx.service, "lp-1"), null);
  });
});

test("R3 legitimate progress prints captured bytes", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("approved-label-v1");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    writeFileSync(file, Buffer.from("file-changed-after-submit"));
    const inst = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(inst.code, 0);
    assert.equal(inst.body.state, "printed");
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "printed");
    assert.equal(st.body.printed_digest, digestOf(bytes));
    const printer = await printerOf(ctx.service, "lp-1");
    assert.equal(printer.digest, digestOf(bytes));
    assert.deepEqual(Buffer.from(printer.label_b64, "base64"), bytes);
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R4 printed job is not printed again", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("once-only");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(inst.code, 0);
    const again = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(again.code, 0);
    assert.equal(again.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(again.body.printed_digest, digestOf(bytes));
    const cancelled = await desk(ctx.stateDir, ctx.service, ["cancel", prep.body.job_id]);
    assert.notEqual(cancelled.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "printed");
    assert.equal(st.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(st.body.printed_digest, digestOf(bytes));
    assert.equal((await effects(ctx.control)).count, 1);
  });
  await session(async (ctx) => {
    const bytes = Buffer.from("crash-after-record");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const crashed = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id], "after-record-before-exit");
    assert.equal(crashed.signal, "SIGKILL");
    assert.equal((await effects(ctx.control)).count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "printed");
    assert.equal(st.body.printed_digest, digestOf(bytes));
    const again = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(again.code, 0);
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R5 crash after accept does not print twice", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("crash-after-accept");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const crashed = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id], "after-accept-before-record");
    assert.equal(crashed.signal, "SIGKILL");
    const fx = await effects(ctx.control);
    assert.equal(fx.count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "dispatch_saved");
    assert.equal(st.body.dispatch_id, fx.effects[0].dispatch_id);
    const retry = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(retry.code, 0);
    assert.equal(retry.body.state, "printed");
    const fx2 = await effects(ctx.control);
    assert.equal(fx2.count, 1);
    assert.equal(fx2.effects[0].dispatch_id, st.body.dispatch_id);
    const done = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(done.body.state, "printed");
    assert.equal(done.body.printed_digest, digestOf(bytes));
    assert.equal(done.body.dispatch_id, st.body.dispatch_id);
  });
});

test("R6 lost reply is reconciled on the same job", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("lost-reply");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    await setFault(ctx.control, { drop_reply: true, delay_ms: 0 });
    const inst = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    const fx = await effects(ctx.control);
    assert.equal(fx.count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.ok(st.body.state === "dispatch_saved" || st.body.state === "printed");
    assert.equal(st.body.dispatch_id, fx.effects[0].dispatch_id);
    if (st.body.state === "dispatch_saved") assert.notEqual(inst.code, 0);
    const rec = await desk(ctx.stateDir, ctx.service, ["reconcile", prep.body.job_id]);
    assert.equal(rec.code, 0);
    assert.equal(rec.body.state, "printed");
    assert.notEqual(rec.body.state, "not_sent");
    assert.equal(rec.body.job_id, prep.body.job_id);
    const done = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(done.body.state, "printed");
    assert.equal(done.body.printed_digest, digestOf(bytes));
    assert.equal(done.body.dispatch_id, fx.effects[0].dispatch_id);
    const again = await desk(ctx.stateDir, ctx.service, ["reconcile", prep.body.job_id]);
    assert.equal(again.code, 0);
    assert.equal(again.body.state, "printed");
    assert.equal(again.body.printed_digest, digestOf(bytes));
    const fx2 = await effects(ctx.control);
    assert.equal(fx2.count, 1);
    assert.equal(fx2.effects[0].dispatch_id, fx.effects[0].dispatch_id);
    assert.equal(fx2.effects[0].job_id, prep.body.job_id);
  });
});

test("R7 conflicting notice does not overwrite", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("notice-conflict");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(inst.code, 0);
    const note = writeNotice(ctx.stateDir, {
      notice_id: "nt-late",
      dispatch_id: inst.body.dispatch_id,
      printer_id: "lp-1",
      job_id: prep.body.job_id,
      digest: digestOf(Buffer.from("older-label")),
      outcome: "rejected",
    });
    const late = await desk(ctx.stateDir, ctx.service, ["notice", note]);
    assert.equal(late.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "printed");
    assert.equal(st.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(st.body.printed_digest, digestOf(bytes));
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R8 success and refusal JSON protocol", OPTS, async () => {
  await session(async (ctx) => {
    const refused = await desk(ctx.stateDir, ctx.service, ["status", "missing-job"]);
    assert.notEqual(refused.code, 0);
    assert.equal(typeof refused.body.error, "string");
    assert.ok(refused.body.error.length > 0);
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, Buffer.from("protocol"));
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    assert.equal(JSON.parse(prep.stdout).job_id, prep.body.job_id);
  });
});

test("R9 unrelated writer file is preserved", OPTS, async () => {
  await session(async (ctx) => {
    const marker = path.join(ctx.stateDir, "unrelated-marker.txt");
    const writer = spawn(process.execPath, ["-e", "require(\"fs\").writeFileSync(process.argv[1], \"keep-me\")", marker], { stdio: "ignore" });
    const writerCode = await new Promise((resolve) => writer.on("close", resolve));
    assert.equal(writerCode, 0);
    const heldBytes = Buffer.from("beside-writer");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, heldBytes);
    const held = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(held.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["hold", held.body.job_id])).code, 0);
    writeFileSync(file, Buffer.from("other-job"));
    const other = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(other.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["dispatch", other.body.job_id])).code, 0);
    assert.equal(readFileSync(marker, "utf8"), "keep-me");
    const stHeld = await desk(ctx.stateDir, ctx.service, ["status", held.body.job_id]);
    const stOther = await desk(ctx.stateDir, ctx.service, ["status", other.body.job_id]);
    assert.equal(stHeld.body.state, "held");
    assert.equal(stHeld.body.digest, digestOf(heldBytes));
    assert.equal(stOther.body.state, "printed");
  });
});

test("R10 reprint links a new job and does not disturb the original", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("reprint-me");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["dispatch", prep.body.job_id]);
    assert.equal(inst.code, 0);
    const before = await effects(ctx.control);
    assert.equal(before.count, 1);
    const rep = await desk(ctx.stateDir, ctx.service, ["reprint", prep.body.job_id]);
    assert.equal(rep.code, 0);
    assert.notEqual(rep.body.job_id, prep.body.job_id);
    assert.equal(rep.body.reprint_of, prep.body.job_id);
    assert.equal(rep.body.state, "queued");
    assert.equal(rep.body.digest, digestOf(bytes));
    assert.equal((await effects(ctx.control)).count, 1);
    const created = await desk(ctx.stateDir, ctx.service, ["status", rep.body.job_id]);
    assert.equal(created.body.state, "queued");
    assert.equal(created.body.reprint_of, prep.body.job_id);
    assert.equal(created.body.dispatch_id, null);
    assert.equal(created.body.printed_digest, null);
    assert.equal(created.body.digest, digestOf(bytes));
    assert.deepEqual(Buffer.from(created.body.label_b64, "base64"), bytes);
    const src = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(src.body.state, "printed");
    assert.equal(src.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(src.body.printed_digest, digestOf(bytes));
    assert.equal(src.body.digest, digestOf(bytes));
    assert.equal(src.body.reprint_of, null);
    assert.deepEqual(Buffer.from(src.body.label_b64, "base64"), bytes);
    const printed = await desk(ctx.stateDir, ctx.service, ["dispatch", rep.body.job_id]);
    assert.equal(printed.code, 0);
    assert.equal(printed.body.state, "printed");
    assert.notEqual(printed.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(printed.body.printed_digest, digestOf(bytes));
    const fx = await effects(ctx.control);
    assert.equal(fx.count, 2);
    assert.equal(fx.effects[0].dispatch_id, inst.body.dispatch_id);
    assert.equal(fx.effects[0].job_id, prep.body.job_id);
    assert.deepEqual(Buffer.from(fx.effects[0].label_b64, "base64"), bytes);
    assert.equal(fx.effects[1].dispatch_id, printed.body.dispatch_id);
    assert.equal(fx.effects[1].job_id, rep.body.job_id);
    assert.deepEqual(Buffer.from(fx.effects[1].label_b64, "base64"), bytes);
    const again = await desk(ctx.stateDir, ctx.service, ["dispatch", rep.body.job_id]);
    assert.equal(again.code, 0);
    assert.equal((await effects(ctx.control)).count, 2);
    const srcAfter = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(srcAfter.body.state, "printed");
    assert.equal(srcAfter.body.dispatch_id, inst.body.dispatch_id);
    assert.equal(srcAfter.body.printed_digest, digestOf(bytes));
    assert.equal(srcAfter.body.reprint_of, null);
  });
  await session(async (ctx) => {
    const bytes = Buffer.from("not-yet-printed");
    const file = path.join(ctx.stateDir, "label.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["submit", "lp-1", file]);
    assert.equal(prep.code, 0);
    const rep = await desk(ctx.stateDir, ctx.service, ["reprint", prep.body.job_id]);
    assert.notEqual(rep.code, 0);
    assert.equal((await effects(ctx.control)).count, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status", prep.body.job_id]);
    assert.equal(st.body.state, "queued");
    assert.equal(st.body.digest, digestOf(bytes));
    assert.equal(st.body.dispatch_id, null);
  });
});
