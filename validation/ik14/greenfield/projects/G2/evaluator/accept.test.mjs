import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";

const PROJECT_DIR = process.env.PROJECT_DIR;
const STUB_DIR = process.env.STUB_DIR;
const OPTS = { timeout: 20000 };

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
  const stateDir = mkdtempSync(path.join(tmpdir(), "g2-"));
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

async function deviceOf(service, id) {
  const res = await fetch(service + "/v1/devices/" + id);
  if (res.status === 404) return null;
  assert.equal(res.status, 200);
  return await res.json();
}

function writeCb(dir, obj, name) {
  const file = path.join(dir, name || "callback.json");
  writeFileSync(file, JSON.stringify(obj));
  return file;
}

test("R1 persistence across restart", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("persist-me");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal(prep.body.digest, digestOf(bytes));
    const again = await desk(ctx.stateDir, ctx.service, ["show"]);
    assert.equal(again.code, 0);
    assert.equal(again.body.proposal_id, prep.body.proposal_id);
    assert.equal(again.body.digest, prep.body.digest);
    assert.deepEqual(Buffer.from(again.body.bytes_b64, "base64"), bytes);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.code, 0);
    assert.equal(st.body.proposal_id, prep.body.proposal_id);
    assert.equal(st.body.digest, prep.body.digest);
    assert.equal(st.body.install, "none");
  });
});

test("R2 prepare presents exact bytes", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from([0, 255, 10, 65]);
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    writeFileSync(file, Buffer.from("mutated-source"));
    const shown = await desk(ctx.stateDir, ctx.service, ["show"]);
    assert.equal(shown.code, 0);
    assert.equal(shown.body.digest, digestOf(bytes));
    assert.deepEqual(Buffer.from(shown.body.bytes_b64, "base64"), bytes);
  });
});

test("R3 approval binds presented digest", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("approve-me");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    const wrong = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, "ab"]);
    assert.notEqual(wrong.code, 0);
    assert.equal(typeof wrong.body.error, "string");
    assert.ok(wrong.body.error.length > 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.approval, "none");
    assert.equal(st.body.approved_digest, null);
    const good = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest]);
    assert.equal(good.code, 0);
    assert.equal(good.body.approval, "approved");
    assert.equal(good.body.approved_digest, prep.body.digest);
  });
});

test("R4 installed bytes match approved configuration", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("approved-config-v1");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    const ap = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest]);
    assert.equal(ap.code, 0);
    writeFileSync(file, Buffer.from("file-changed-after-approval"));
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(inst.code, 0);
    assert.equal(inst.body.install, "installed");
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, digestOf(bytes));
    const dev = await deviceOf(ctx.service, "dev-1");
    assert.equal(dev.digest, digestOf(bytes));
    assert.deepEqual(Buffer.from(dev.config_b64, "base64"), bytes);
  });
});

test("R5 legitimate progress is not refused", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("progress");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    const ap = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest]);
    assert.equal(ap.code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(inst.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.code, 0);
    assert.equal(st.body.install, "installed");
  });
});

test("R6 repeated callback keeps terminal outcome", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("callback-repeat");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(inst.code, 0);
    const cb = {
      callback_id: "cb-1",
      request_id: inst.body.request_id,
      device_id: "dev-1",
      digest: prep.body.digest,
      outcome: "installed",
    };
    const cbFile = writeCb(ctx.stateDir, cb);
    const first = await desk(ctx.stateDir, ctx.service, ["callback", cbFile]);
    const second = await desk(ctx.stateDir, ctx.service, ["callback", cbFile]);
    assert.equal(first.code, 0);
    assert.equal(second.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, prep.body.digest);
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R7 conflicting callback does not overwrite terminal outcome", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("callback-conflict");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(inst.code, 0);
    const cbFile = writeCb(ctx.stateDir, {
      callback_id: "cb-late",
      request_id: inst.body.request_id,
      device_id: "dev-1",
      digest: digestOf(Buffer.from("older-bytes")),
      outcome: "rejected",
    });
    const late = await desk(ctx.stateDir, ctx.service, ["callback", cbFile]);
    assert.equal(late.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, prep.body.digest);
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R8 crash after accept reconciles without a second effect", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("crash-after-accept");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const crashed = await desk(ctx.stateDir, ctx.service, ["install"], "after-accept-before-record");
    assert.equal(crashed.signal, "SIGKILL");
    const fx = await effects(ctx.control);
    assert.equal(fx.count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "attempt_saved");
    assert.equal(st.body.request_id, fx.effects[0].request_id);
    const retry = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(retry.code, 0);
    assert.equal(retry.body.install, "installed");
    const fx2 = await effects(ctx.control);
    assert.equal(fx2.count, 1);
    assert.equal(fx2.effects[0].request_id, st.body.request_id);
    const done = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(done.body.install, "installed");
    assert.equal(done.body.installed_digest, prep.body.digest);
  });
});

test("R9 crash before request still completes once", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("crash-before-request");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const crashed = await desk(ctx.stateDir, ctx.service, ["install"], "after-attempt-saved-before-request");
    assert.equal(crashed.signal, "SIGKILL");
    assert.equal((await effects(ctx.control)).count, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "attempt_saved");
    assert.equal(typeof st.body.request_id, "string");
    assert.ok(st.body.request_id.length > 0);
    const retry = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(retry.code, 0);
    assert.equal((await effects(ctx.control)).count, 1);
    const done = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(done.body.install, "installed");
    assert.equal(done.body.installed_digest, prep.body.digest);
  });
});

test("R10 crash after durable record keeps outcome", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("crash-after-record");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const crashed = await desk(ctx.stateDir, ctx.service, ["install"], "after-record-before-exit");
    assert.equal(crashed.signal, "SIGKILL");
    assert.equal((await effects(ctx.control)).count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, prep.body.digest);
    const again = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(again.code, 0);
    assert.equal((await effects(ctx.control)).count, 1);
  });
});

test("R11 single device binding", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("one-device");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-a", file]);
    assert.equal(prep.code, 0);
    const other = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-b", file]);
    assert.notEqual(other.code, 0);
    const shown = await desk(ctx.stateDir, ctx.service, ["show"]);
    assert.equal(shown.code, 0);
    assert.equal(shown.body.device_id, "dev-a");
    assert.deepEqual(Buffer.from(shown.body.bytes_b64, "base64"), bytes);
  });
});

test("R12 success and refusal JSON protocol", OPTS, async () => {
  await session(async (ctx) => {
    const refused = await desk(ctx.stateDir, ctx.service, ["approve", "missing", "ab"]);
    assert.notEqual(refused.code, 0);
    assert.equal(typeof refused.body.error, "string");
    assert.ok(refused.body.error.length > 0);
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, Buffer.from("protocol"));
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal(JSON.parse(prep.stdout).proposal_id, prep.body.proposal_id);
  });
});

test("R13 unrelated writer file is preserved", OPTS, async () => {
  await session(async (ctx) => {
    const marker = path.join(ctx.stateDir, "unrelated-marker.txt");
    const writer = spawn(process.execPath, ["-e", "require(\"fs\").writeFileSync(process.argv[1], \"keep-me\")", marker], { stdio: "ignore" });
    const writerCode = await new Promise((resolve) => writer.on("close", resolve));
    assert.equal(writerCode, 0);
    const bytes = Buffer.from("beside-writer");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["install"])).code, 0);
    assert.equal(readFileSync(marker, "utf8"), "keep-me");
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.digest, digestOf(bytes));
  });
});

test("R14 lost reply is reconciled without misattribution", OPTS, async () => {
  await session(async (ctx) => {
    const bytes = Buffer.from("lost-reply");
    const file = path.join(ctx.stateDir, "cfg.bin");
    writeFileSync(file, bytes);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    await setFault(ctx.control, { drop_reply: true, delay_ms: 0 });
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    const fx = await effects(ctx.control);
    assert.equal(fx.count, 1);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.ok(st.body.install === "attempt_saved" || st.body.install === "installed");
    assert.equal(st.body.request_id, fx.effects[0].request_id);
    if (st.body.install === "attempt_saved") assert.notEqual(inst.code, 0);
    const rec = await desk(ctx.stateDir, ctx.service, ["reconcile"]);
    assert.equal(rec.code, 0);
    assert.equal(rec.body.install, "installed");
    assert.notEqual(rec.body.install, "not_sent");
    const done = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(done.body.install, "installed");
    assert.equal(done.body.installed_digest, prep.body.digest);
    const fx2 = await effects(ctx.control);
    assert.equal(fx2.count, 1);
    assert.equal(fx2.effects[0].request_id, fx.effects[0].request_id);
  });
});

test("R15 replace after review clears approval", OPTS, async () => {
  await session(async (ctx) => {
    const first = Buffer.from("reviewed-v1");
    const second = Buffer.from("reviewed-v2");
    const file = path.join(ctx.stateDir, "cfg.bin");
    const next = path.join(ctx.stateDir, "next.bin");
    writeFileSync(file, first);
    writeFileSync(next, second);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    const shown = await desk(ctx.stateDir, ctx.service, ["show"]);
    assert.equal(shown.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const rep = await desk(ctx.stateDir, ctx.service, ["replace", prep.body.proposal_id, next]);
    assert.equal(rep.code, 0);
    assert.equal(rep.body.proposal_id, prep.body.proposal_id);
    assert.equal(rep.body.approval, "none");
    assert.equal(rep.body.approved_digest, null);
    writeFileSync(next, Buffer.from("mutated-after-replace"));
    const after = await desk(ctx.stateDir, ctx.service, ["show"]);
    assert.equal(after.body.proposal_id, prep.body.proposal_id);
    assert.equal(after.body.device_id, "dev-1");
    assert.equal(after.body.digest, digestOf(second));
    assert.equal(after.body.approval, "none");
    assert.equal(after.body.approved_digest, null);
    assert.deepEqual(Buffer.from(after.body.bytes_b64, "base64"), second);
  });
  await session(async (ctx) => {
    const bytes = Buffer.from("already-installed");
    const file = path.join(ctx.stateDir, "cfg.bin");
    const next = path.join(ctx.stateDir, "next.bin");
    writeFileSync(file, bytes);
    writeFileSync(next, Buffer.from("should-not-land"));
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["install"])).code, 0);
    const before = await effects(ctx.control);
    const rep = await desk(ctx.stateDir, ctx.service, ["replace", prep.body.proposal_id, next]);
    assert.notEqual(rep.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, digestOf(bytes));
    assert.equal((await effects(ctx.control)).count, before.count);
    const dev = await deviceOf(ctx.service, "dev-1");
    assert.deepEqual(Buffer.from(dev.config_b64, "base64"), bytes);
  });
});

test("R16 old approval cannot authorize replacement bytes", OPTS, async () => {
  await session(async (ctx) => {
    const first = Buffer.from("old-approved");
    const second = Buffer.from("new-unapproved");
    const file = path.join(ctx.stateDir, "cfg.bin");
    const next = path.join(ctx.stateDir, "next.bin");
    writeFileSync(file, first);
    writeFileSync(next, second);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    const oldDigest = prep.body.digest;
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, oldDigest])).code, 0);
    const rep = await desk(ctx.stateDir, ctx.service, ["replace", prep.body.proposal_id, next]);
    assert.equal(rep.code, 0);
    const old = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, oldDigest]);
    assert.notEqual(old.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.approval, "none");
    assert.equal(st.body.approved_digest, null);
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.notEqual(inst.code, 0);
    assert.equal((await effects(ctx.control)).count, 0);
    assert.equal(await deviceOf(ctx.service, "dev-1"), null);
  });
});

test("R17 reapproval of replacement installs new bytes", OPTS, async () => {
  await session(async (ctx) => {
    const first = Buffer.from("before-correction");
    const second = Buffer.from("after-correction");
    const file = path.join(ctx.stateDir, "cfg.bin");
    const next = path.join(ctx.stateDir, "next.bin");
    writeFileSync(file, first);
    writeFileSync(next, second);
    const prep = await desk(ctx.stateDir, ctx.service, ["prepare", "dev-1", file]);
    assert.equal(prep.code, 0);
    assert.equal((await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, prep.body.digest])).code, 0);
    const rep = await desk(ctx.stateDir, ctx.service, ["replace", prep.body.proposal_id, next]);
    assert.equal(rep.code, 0);
    writeFileSync(next, Buffer.from("mutated-after-capture"));
    const ap = await desk(ctx.stateDir, ctx.service, ["approve", prep.body.proposal_id, digestOf(second)]);
    assert.equal(ap.code, 0);
    const inst = await desk(ctx.stateDir, ctx.service, ["install"]);
    assert.equal(inst.code, 0);
    const st = await desk(ctx.stateDir, ctx.service, ["status"]);
    assert.equal(st.body.install, "installed");
    assert.equal(st.body.installed_digest, digestOf(second));
    const dev = await deviceOf(ctx.service, "dev-1");
    assert.deepEqual(Buffer.from(dev.config_b64, "base64"), second);
    assert.notDeepEqual(Buffer.from(dev.config_b64, "base64"), first);
  });
});
