import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import http from "node:http";

export const PROJECT_DIR = process.env.PROJECT_DIR;
export const STUB_DIR = process.env.STUB_DIR;

export function assertEnv() {
  if (!PROJECT_DIR) throw new Error("PROJECT_DIR is required");
  if (!STUB_DIR) throw new Error("STUB_DIR is required");
}

function httpJson(base, pathname, { method = "GET", body } = {}) {
  return new Promise((resolve, reject) => {
    const u = new URL(pathname, base);
    const payload = body == null ? null : JSON.stringify(body);
    const req = http.request(
      {
        hostname: u.hostname,
        port: u.port,
        path: `${u.pathname}${u.search}`,
        method,
        headers: payload
          ? { "content-type": "application/json", "content-length": Buffer.byteLength(payload) }
          : undefined,
      },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const text = Buffer.concat(chunks).toString("utf8");
          let parsed = null;
          if (text) {
            try { parsed = JSON.parse(text); } catch { parsed = text; }
          }
          resolve({ status: res.statusCode, body: parsed });
        });
      },
    );
    req.on("error", reject);
    if (payload) req.end(payload);
    else req.end();
  });
}

async function waitForReady(file, child) {
  const start = Date.now();
  for (;;) {
    if (child.exitCode != null) {
      throw new Error(`stub exited ${child.exitCode} before ready`);
    }
    try {
      const text = await fs.readFile(file, "utf8");
      if (text) return JSON.parse(text);
    } catch (err) {
      if (err.code !== "ENOENT") throw err;
    }
    if (Date.now() - start > 5000) throw new Error("stub ready timeout");
    await new Promise((r) => setTimeout(r, 20));
  }
}

export async function withFixture(fn) {
  assertEnv();
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "g3-eval-"));
  const readyFile = path.join(dir, "ready.json");
  const stub = spawn(process.execPath, [path.join(STUB_DIR, "server.mjs"), readyFile], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stubErr = "";
  stub.stderr.on("data", (d) => { stubErr += d; });
  const children = new Set();
  try {
    const ready = await waitForReady(readyFile, stub);
    const service = `http://127.0.0.1:${ready.publicPort}`;
    const control = `http://127.0.0.1:${ready.controlPort}`;
    const stateDir = path.join(dir, "state");
    await fs.mkdir(stateDir);
    const fx = {
      dir,
      stateDir,
      service,
      control,
      async config(body) {
        const res = await httpJson(control, "/control/config", { method: "POST", body });
        if (res.status !== 200) throw new Error(`config ${res.status} ${JSON.stringify(res.body)}`);
        return res.body;
      },
      async state() {
        const res = await httpJson(control, "/control/state");
        if (res.status !== 200) throw new Error(`state ${res.status}`);
        return res.body;
      },
      async release() {
        return httpJson(control, "/control/release", { method: "POST", body: {} });
      },
      async seed(idempotency_key, record) {
        const res = await httpJson(control, "/control/seed", {
          method: "POST",
          body: { idempotency_key, record },
        });
        if (res.status !== 200) throw new Error(`seed ${res.status} ${JSON.stringify(res.body)}`);
        return res.body;
      },
      async lookup(id) {
        return httpJson(service, `/v1/deliveries/${encodeURIComponent(id)}`);
      },
      posts(snapshot, record) {
        return snapshot.requests.filter((r) => r.method === "POST" && (record == null || r.record === record));
      },
      effects(snapshot, record) {
        return snapshot.effects.filter((e) => record == null || e.record === record);
      },
      cmd(args, envExtra = {}) {
        const env = { ...process.env, STATE_DIR: stateDir, SERVICE_URL: service };
        delete env.CRASH_AT;
        delete env.DEFECT;
        for (const [k, v] of Object.entries(envExtra)) {
          if (v == null) delete env[k];
          else env[k] = v;
        }
        return runChild(path.join(PROJECT_DIR, "run"), args, env, children);
      },
    };
    await fn(fx);
  } catch (err) {
    if (stubErr) err.message = `${err.message}\nstub: ${stubErr}`;
    throw err;
  } finally {
    for (const child of children) {
      if (child.exitCode == null && child.signalCode == null) child.kill("SIGKILL");
    }
    if (stub.exitCode == null) stub.kill("SIGKILL");
    await fs.rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}

function runChild(command, args, env, children) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { env, cwd: PROJECT_DIR });
    children.add(child);
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => { stdout += d; });
    child.stderr.on("data", (d) => { stderr += d; });
    child.on("close", (code, signal) => {
      children.delete(child);
      resolve({ code, signal, stdout, stderr });
    });
  });
}

export function killed(result) {
  return result.signal === "SIGKILL" || result.code === 137 || result.code === 9;
}

export function jsonOf(result) {
  const text = result.stdout.trim();
  if (!text) {
    throw new Error(`empty stdout code=${result.code} signal=${result.signal} stderr=${result.stderr}`);
  }
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new Error(`stdout not one JSON object: ${text}\nstderr=${result.stderr}`);
  }
}

export function expectOk(result) {
  if (result.code !== 0 || result.signal) {
    throw new Error(`expected exit 0, got code=${result.code} signal=${result.signal} stdout=${result.stdout} stderr=${result.stderr}`);
  }
  const obj = jsonOf(result);
  if (obj.error) throw new Error(`expected success, got ${result.stdout}`);
  return obj;
}

export function expectErr(result, code) {
  if (result.code === 0) throw new Error(`expected refusal, got ${result.stdout}`);
  const obj = jsonOf(result);
  if (obj.error !== code) {
    throw new Error(`expected error ${code}, got ${result.stdout} stderr=${result.stderr}`);
  }
  return obj;
}

export async function waitUntil(fn, ms = 5000) {
  const start = Date.now();
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() - start > ms) throw new Error("timed out waiting");
    await new Promise((r) => setTimeout(r, 20));
  }
}

export function assertStatus(obj, fields) {
  for (const key of ["import_id", "phase", "dispatch", "attempts_failed", "attempts_remaining", "destination_id", "last_error"]) {
    if (!(key in obj)) throw new Error(`missing ${key} in ${JSON.stringify(obj)}`);
  }
  if (typeof obj.attempts_failed !== "number" || typeof obj.attempts_remaining !== "number") {
    throw new Error(`attempt fields must be numbers: ${JSON.stringify(obj)}`);
  }
  if (obj.attempts_remaining !== 3 - obj.attempts_failed) {
    throw new Error(`attempts_remaining mismatch: ${JSON.stringify(obj)}`);
  }
  for (const [k, v] of Object.entries(fields)) {
    if (obj[k] !== v) throw new Error(`expected ${k}=${JSON.stringify(v)} in ${JSON.stringify(obj)}`);
  }
}
