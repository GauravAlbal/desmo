import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import crypto from "node:crypto";

const MAX_FAILED = 3;
const DEFECT = process.env.DEFECT || "";
const STATE_DIR = process.env.STATE_DIR;
const SERVICE_URL = process.env.SERVICE_URL;

function emit(obj, code = 0) {
  process.stdout.write(`${JSON.stringify(obj)}\n`);
  process.exit(code);
}

function crashIf(label) {
  if (process.env.CRASH_AT === label) {
    process.kill(process.pid, "SIGKILL");
  }
}

function validId(s) {
  return typeof s === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(s);
}

function validRecord(s) {
  return typeof s === "string" && [...s].length >= 1 && [...s].length <= 256;
}

function view(imp) {
  return {
    import_id: imp.import_id,
    phase: imp.phase,
    dispatch: imp.dispatch,
    attempts_failed: imp.attempts_failed,
    attempts_remaining: MAX_FAILED - imp.attempts_failed,
    destination_id: imp.destination_id,
    last_error: imp.last_error,
  };
}

function statePath() {
  return path.join(STATE_DIR, "imports.json");
}

function lockPath() {
  return path.join(STATE_DIR, "lock");
}

async function load() {
  try {
    const text = await fs.readFile(statePath(), "utf8");
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || !parsed.imports) return { imports: {} };
    return parsed;
  } catch (err) {
    if (err.code === "ENOENT") return { imports: {} };
    throw err;
  }
}

async function save(state) {
  await fs.mkdir(STATE_DIR, { recursive: true });
  const tmp = path.join(STATE_DIR, `.imports.${process.pid}.tmp`);
  const fh = await fs.open(tmp, "w");
  try {
    await fh.writeFile(JSON.stringify(state));
    await fh.sync();
  } finally {
    await fh.close();
  }
  await fs.rename(tmp, statePath());
}

async function withLock(fn) {
  await fs.mkdir(STATE_DIR, { recursive: true });
  for (;;) {
    let fh;
    try {
      fh = await fs.open(lockPath(), "wx");
    } catch (err) {
      if (err.code === "EEXIST") {
        await new Promise((r) => setTimeout(r, 5));
        continue;
      }
      throw err;
    }
    try {
      return await fn();
    } finally {
      await fh.close();
      await fs.unlink(lockPath()).catch(() => {});
    }
  }
}

function requestJson(method, pathname, body) {
  return new Promise((resolve, reject) => {
    const u = new URL(pathname, SERVICE_URL);
    const payload = body == null ? null : JSON.stringify(body);
    const req = http.request(
      {
        hostname: u.hostname,
        port: u.port,
        path: `${u.pathname}${u.search}`,
        method,
        headers: payload
          ? {
              "content-type": "application/json",
              "content-length": Buffer.byteLength(payload),
            }
          : undefined,
      },
      (res) => {
        if (method === "POST" && process.env.CRASH_AT === "after-dispatch-before-result") {
          process.kill(process.pid, "SIGKILL");
          return;
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const text = Buffer.concat(chunks).toString("utf8");
          if (method === "POST" && process.env.CRASH_AT === "after-result-before-commit") {
            process.kill(process.pid, "SIGKILL");
            return;
          }
          let parsed = null;
          if (text) {
            try {
              parsed = JSON.parse(text);
            } catch {
              parsed = null;
            }
          }
          resolve({ status: res.statusCode, body: parsed });
        });
      },
    );
    req.on("error", (err) => {
      if (method === "POST" && process.env.CRASH_AT === "after-dispatch-before-result") {
        process.kill(process.pid, "SIGKILL");
        return;
      }
      reject(err);
    });
    if (payload) req.end(payload);
    else req.end();
  });
}

function postDelivery(key, record) {
  return requestJson("POST", "/v1/deliveries", { idempotency_key: key, record });
}

async function lookup(key) {
  const res = await requestJson("GET", `/v1/deliveries/${encodeURIComponent(key)}`);
  if (res.status === 404) return null;
  if (res.status >= 500 || !res.body) {
    const err = new Error("lookup_unavailable");
    err.code = "LOOKUP";
    throw err;
  }
  if (res.body.found === false) return null;
  return res.body;
}

function sameImport(a, b) {
  return a.phase === b.phase
    && a.dispatch === b.dispatch
    && a.attempts_failed === b.attempts_failed
    && a.destination_id === b.destination_id
    && a.last_error === b.last_error
    && a.record === b.record
    && a.import_id === b.import_id;
}

function applyStale(imp, reply) {
  const next = structuredClone(imp);
  if (reply.status === 409 || reply.body?.error === "conflict" || reply.body?.outcome === "conflict") {
    next.phase = "failed";
    next.dispatch = "none";
    next.last_error = "conflict";
    next.destination_id = null;
    return next;
  }
  if (reply.status === 422 || reply.body?.outcome === "rejected") {
    next.attempts_failed += 1;
    next.last_error = "rejected";
    next.dispatch = "none";
    next.destination_id = null;
    next.phase = next.attempts_failed >= MAX_FAILED ? "failed" : "ready";
    return next;
  }
  if (reply.status === 200 && reply.body?.outcome === "stored") {
    next.phase = "delivered";
    next.dispatch = "confirmed";
    next.destination_id = reply.body.destination_id;
    next.last_error = null;
    return next;
  }
  return next;
}

async function correctApply(imp, reply) {
  if (imp.phase === "delivered") return imp;
  let found;
  try {
    found = await lookup(imp.import_id);
  } catch {
    return imp;
  }
  if (found && found.record === imp.record) {
    imp.phase = "delivered";
    imp.dispatch = "confirmed";
    imp.destination_id = found.destination_id;
    imp.last_error = null;
    return imp;
  }
  if (found && found.record !== imp.record) {
    imp.phase = "failed";
    imp.dispatch = "none";
    imp.last_error = "conflict";
    imp.destination_id = null;
    return imp;
  }
  if (reply && (reply.status === 409 || reply.body?.error === "conflict" || reply.body?.outcome === "conflict")) {
    imp.phase = "failed";
    imp.dispatch = "none";
    imp.last_error = "conflict";
    imp.destination_id = null;
    return imp;
  }
  if (reply && (reply.status === 422 || reply.body?.outcome === "rejected")) {
    imp.attempts_failed += 1;
    imp.last_error = "rejected";
    imp.dispatch = "none";
    imp.destination_id = null;
    imp.phase = imp.attempts_failed >= MAX_FAILED ? "failed" : "ready";
    return imp;
  }
  if (reply && reply.status === 200 && reply.body?.outcome === "stored") {
    imp.phase = "delivered";
    imp.dispatch = "confirmed";
    imp.destination_id = reply.body.destination_id;
    imp.last_error = null;
    return imp;
  }
  return imp;
}

async function commit(before, reply) {
  return withLock(async () => {
    const state = await load();
    const current = state.imports[before.import_id];
    if (!current) return { error: "not_found" };
    if (DEFECT === "late-overwrite" && !sameImport(current, before)) {
      state.imports[before.import_id] = applyStale(before, reply);
      await save(state);
      return view(state.imports[before.import_id]);
    }
    if (current.phase === "delivered") {
      return view(current);
    }
    await correctApply(current, reply);
    if (current.attempts_failed < before.attempts_failed) current.attempts_failed = before.attempts_failed;
    state.imports[current.import_id] = current;
    await save(state);
    return view(current);
  });
}

async function markDelivered(id, found) {
  return withLock(async () => {
    const state = await load();
    const imp = state.imports[id];
    if (!imp) return { error: "not_found" };
    if (imp.phase === "delivered") return view(imp);
    if (imp.attempts_failed > (imp.attempts_failed || 0)) {
      /* keep */
    }
    imp.phase = "delivered";
    imp.dispatch = "confirmed";
    imp.destination_id = found.destination_id;
    imp.last_error = null;
    await save(state);
    return view(imp);
  });
}

async function saveImport(imp) {
  return withLock(async () => {
    const state = await load();
    const current = state.imports[imp.import_id];
    if (current && current.phase === "delivered" && DEFECT !== "late-overwrite") {
      return view(current);
    }
    if (current && current.attempts_failed > imp.attempts_failed && DEFECT !== "late-overwrite") {
      imp.attempts_failed = current.attempts_failed;
    }
    state.imports[imp.import_id] = imp;
    await save(state);
    return view(imp);
  });
}

function applyRetryReset(imp) {
  if (DEFECT !== "retry-reset") return;
  if (imp.last_error === "conflict" || (imp.phase === "failed" && imp.last_error !== "rejected")) return;
  if (imp.attempts_failed) imp.attempts_failed = 0;
  if (imp.phase === "failed") {
    imp.phase = "ready";
    imp.dispatch = "none";
    imp.last_error = null;
    imp.destination_id = null;
  }
}

async function openImport(id) {
  return withLock(async () => {
    const state = await load();
    const imp = state.imports[id];
    if (!imp) return { missing: true };
    applyRetryReset(imp);
    await save(state);
    return { imp: structuredClone(imp) };
  });
}

async function submit(id, record) {
  if (!validId(id)) return { error: "invalid_input", field: "import_id" };
  if (!validRecord(record)) return { error: "invalid_input", field: "record" };
  return withLock(async () => {
    const state = await load();
    const existing = state.imports[id];
    if (existing) {
      if (existing.record !== record) return { error: "conflict" };
      return view(existing);
    }
    const imp = {
      import_id: id,
      record,
      phase: "ready",
      dispatch: "none",
      attempts_failed: 0,
      destination_id: null,
      last_error: null,
    };
    state.imports[id] = imp;
    await save(state);
    return view(imp);
  });
}

async function status(id) {
  if (!validId(id)) return { error: "invalid_input", field: "import_id" };
  const state = await load();
  const imp = state.imports[id];
  if (!imp) return { error: "not_found" };
  return view(imp);
}

async function pause(id) {
  if (!validId(id)) return { error: "invalid_input", field: "import_id" };
  return withLock(async () => {
    const state = await load();
    const imp = state.imports[id];
    if (!imp) return { error: "not_found" };
    if (imp.phase === "delivered" || imp.phase === "failed") return { error: "not_pausable" };
    if (imp.phase !== "paused") imp.phase = "paused";
    await save(state);
    return view(imp);
  });
}

async function resume(id) {
  if (!validId(id)) return { error: "invalid_input", field: "import_id" };
  return withLock(async () => {
    const state = await load();
    const imp = state.imports[id];
    if (!imp) return { error: "not_found" };
    if (imp.phase !== "paused") return { error: "not_paused" };
    imp.phase = imp.dispatch === "in_flight" ? "uncertain" : "ready";
    await save(state);
    return view(imp);
  });
}

async function reconcileFound(id) {
  let found;
  try {
    found = await lookup(id);
  } catch {
    return null;
  }
  if (found && found.record) {
    const state = await load();
    const imp = state.imports[id];
    if (imp && found.record !== imp.record) {
      imp.phase = "failed";
      imp.dispatch = "none";
      imp.last_error = "conflict";
      imp.destination_id = null;
      return saveImport(imp);
    }
    if (found.record === imp.record) return markDelivered(id, found);
  }
  return null;
}

async function advance(id) {
  if (!validId(id)) return { error: "invalid_input", field: "import_id" };
  const opened = await openImport(id);
  if (opened.missing) return { error: "not_found" };
  crashIf("after-record-before-dispatch");

  let imp = opened.imp;

  if (DEFECT === "duplicate-effect" && imp.phase === "delivered") {
    await postDelivery(`${id}~dup~${crypto.randomBytes(4).toString("hex")}`, imp.record);
    return view(imp);
  }

  if (imp.phase === "delivered" || imp.phase === "failed") return view(imp);

  if (imp.phase === "paused" && imp.dispatch !== "in_flight") return view(imp);

  if (imp.phase === "paused" && imp.dispatch === "in_flight") {
    const done = await reconcileFound(id);
    if (done) return done;
    return view(imp);
  }

  if (imp.dispatch === "in_flight" || imp.phase === "uncertain") {
    if (DEFECT === "duplicate-effect") {
      const done = await reconcileFound(id);
      const key = `${id}~dup~${crypto.randomBytes(4).toString("hex")}`;
      try {
        await postDelivery(key, imp.record);
      } catch {
        /* the extra submit can still have been stored before the reply was lost */
      }
      if (done) return done;
      return view(imp);
    }
    const done = await reconcileFound(id);
    if (done) return done;
    let found;
    try {
      found = await lookup(id);
    } catch {
      return view(imp);
    }
    if (found && found.record !== imp.record) {
      imp.phase = "failed";
      imp.dispatch = "none";
      imp.last_error = "conflict";
      imp.destination_id = null;
      return saveImport(imp);
    }
    if (!found) {
      /* unfinished send: fall through and POST the same key */
    }
  }

  if (imp.attempts_failed >= MAX_FAILED) {
    imp.phase = "failed";
    imp.dispatch = "none";
    return saveImport(imp);
  }

  const before = await withLock(async () => {
    const state = await load();
    const cur = state.imports[id];
    if (!cur) return null;
    if (cur.phase === "delivered" || cur.phase === "failed") return structuredClone(cur);
    if (cur.phase === "paused" && cur.dispatch !== "in_flight") return structuredClone(cur);
    cur.phase = "uncertain";
    cur.dispatch = "in_flight";
    await save(state);
    return structuredClone(cur);
  });
  if (!before) return { error: "not_found" };
  if (before.phase === "delivered" || before.phase === "failed" || (before.phase === "paused" && before.dispatch !== "in_flight")) {
    return view(before);
  }

  let reply;
  try {
    reply = await postDelivery(id, before.record);
  } catch {
    if (DEFECT === "lost-reply-misattribution") {
      const cleared = structuredClone(before);
      cleared.phase = "ready";
      cleared.dispatch = "none";
      return saveImport(cleared);
    }
    const done = await reconcileFound(id);
    if (done) return done;
    return view(before);
  }
  return commit(before, reply);
}

function isError(obj) {
  return obj && typeof obj.error === "string";
}

async function main() {
  if (!STATE_DIR || !SERVICE_URL) emit({ error: "invalid_input", field: "env" }, 1);
  const [cmd, id, record] = process.argv.slice(2);
  let result;
  if (cmd === "submit") result = await submit(id, record);
  else if (cmd === "status") result = await status(id);
  else if (cmd === "advance") result = await advance(id);
  else if (cmd === "pause") result = await pause(id);
  else if (cmd === "resume") result = await resume(id);
  else result = { error: "unknown_command" };
  emit(result, isError(result) ? 1 : 0);
}

main().catch((err) => {
  process.stderr.write(`${err.stack || err}\n`);
  emit({ error: "internal" }, 1);
});
