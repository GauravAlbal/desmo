import http from "node:http";
import fs from "node:fs";

const readyPath = process.argv[2];
if (!readyPath) {
  process.stderr.write("usage: node server.mjs <ready-file>\n");
  process.exit(1);
}

const effects = new Map();
const requests = [];
const callbackNotes = [];
let held = [];
let seq = 0;

function defaultConfig() {
  return {
    drop_reply: false,
    hold: false,
    hold_key: null,
    reject: false,
    late_conflict: false,
    delay_ms: 0,
    duplicate_reply: false,
    callback_conflict: false,
    fail_lookup: false,
    force_status: 0,
  };
}

let config = defaultConfig();

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function sendJson(res, status, obj) {
  const payload = JSON.stringify(obj);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function postJson(url, obj) {
  return new Promise((resolve, reject) => {
    let u;
    try {
      u = new URL(url);
    } catch (err) {
      reject(err);
      return;
    }
    const payload = JSON.stringify(obj);
    const req = http.request(
      {
        hostname: u.hostname,
        port: u.port,
        path: `${u.pathname}${u.search}`,
        method: "POST",
        headers: {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(payload),
        },
      },
      (res) => {
        res.resume();
        res.on("end", () => resolve());
      },
    );
    req.on("error", reject);
    req.end(payload);
  });
}

function fireCallbacks(body, payload) {
  if (!config.duplicate_reply && !config.callback_conflict) return;
  const url = body && body.callback_url;
  const sends = [];
  if (config.duplicate_reply) sends.push(payload, payload);
  if (config.callback_conflict) {
    sends.push({ outcome: "rejected", error: "rejected", reason: "invalid_record" });
  }
  if (!url) {
    callbackNotes.push({ skipped: true, count: sends.length });
    return;
  }
  for (const item of sends) {
    callbackNotes.push({ url, outcome: item.outcome });
    postJson(url, item).catch(() => {});
  }
}

function finish(res, status, payload, body) {
  const send = () => {
    if (res.writableEnded || res.destroyed) return;
    sendJson(res, status, payload);
    fireCallbacks(body, payload);
  };
  if (config.drop_reply) {
    res.socket.destroy();
    return;
  }
  const key = body && body.idempotency_key;
  const holdThis = config.hold && (config.hold_key == null || config.hold_key === key);
  if (holdThis) {
    held.push(send);
    return;
  }
  const delay = Number(config.delay_ms) || 0;
  if (delay > 0) {
    setTimeout(send, delay);
    return;
  }
  send();
}

function snapshot() {
  return {
    config,
    effects: [...effects.values()],
    requests,
    held: held.length,
    callback_notes: callbackNotes,
  };
}

const publicServer = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1");
    if (req.method === "GET" && url.pathname.startsWith("/v1/deliveries/")) {
      requests.push({ method: "GET", path: url.pathname });
      if (config.fail_lookup) {
        sendJson(res, 500, { error: "unavailable" });
        return;
      }
      const key = decodeURIComponent(url.pathname.slice("/v1/deliveries/".length));
      const effect = effects.get(key);
      if (!effect) {
        sendJson(res, 404, { found: false });
        return;
      }
      sendJson(res, 200, { found: true, ...effect });
      return;
    }
    if (req.method === "POST" && url.pathname === "/v1/deliveries") {
      let body;
      try {
        body = JSON.parse(await readBody(req));
      } catch {
        sendJson(res, 400, { error: "bad_json" });
        return;
      }
      if (typeof body.idempotency_key !== "string" || typeof body.record !== "string" || body.idempotency_key.length === 0 || body.record.length === 0) {
        sendJson(res, 400, { error: "bad_request" });
        return;
      }
      const key = body.idempotency_key;
      const record = body.record;
      let status = 200;
      let payload;
      if (config.force_status) {
        status = config.force_status;
        payload = { outcome: "error", error: "unavailable" };
      } else if (config.reject) {
        status = 422;
        payload = { outcome: "rejected", error: "rejected", reason: "invalid_record" };
      } else if (effects.has(key)) {
        const existing = effects.get(key);
        if (existing.record !== record) {
          status = 409;
          payload = { outcome: "conflict", error: "conflict" };
        } else if (config.late_conflict) {
          status = 422;
          payload = { outcome: "rejected", error: "rejected", reason: "invalid_record" };
        } else {
          status = 200;
          payload = { outcome: "stored", destination_id: existing.destination_id, idempotency_key: key };
        }
      } else {
        const destination_id = `dst_${++seq}`;
        effects.set(key, { idempotency_key: key, record, destination_id });
        if (config.late_conflict) {
          status = 422;
          payload = { outcome: "rejected", error: "rejected", reason: "invalid_record" };
        } else {
          status = 200;
          payload = { outcome: "stored", destination_id, idempotency_key: key };
        }
      }
      requests.push({
        method: "POST",
        path: "/v1/deliveries",
        idempotency_key: key,
        record,
        status,
      });
      finish(res, status, payload, body);
      return;
    }
    sendJson(res, 404, { error: "not_found" });
  } catch (err) {
    process.stderr.write(`public error: ${err.stack || err}\n`);
    if (!res.headersSent) sendJson(res, 500, { error: "internal" });
  }
});

const controlServer = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://127.0.0.1");
    if (req.method === "GET" && url.pathname === "/control/state") {
      sendJson(res, 200, snapshot());
      return;
    }
    if (req.method === "POST" && url.pathname === "/control/config") {
      const body = JSON.parse(await readBody(req) || "{}");
      config = { ...defaultConfig(), ...body };
      sendJson(res, 200, { ok: true, config });
      return;
    }
    if (req.method === "POST" && url.pathname === "/control/release") {
      const pending = held;
      held = [];
      for (const fn of pending) fn();
      sendJson(res, 200, { released: pending.length });
      return;
    }
    if (req.method === "POST" && url.pathname === "/control/reset") {
      effects.clear();
      requests.length = 0;
      callbackNotes.length = 0;
      held = [];
      seq = 0;
      config = defaultConfig();
      sendJson(res, 200, { ok: true });
      return;
    }
    if (req.method === "POST" && url.pathname === "/control/seed") {
      const body = JSON.parse(await readBody(req) || "{}");
      if (typeof body.idempotency_key !== "string" || typeof body.record !== "string") {
        sendJson(res, 400, { error: "bad_request" });
        return;
      }
      if (effects.has(body.idempotency_key)) {
        sendJson(res, 409, { error: "conflict" });
        return;
      }
      const destination_id = `dst_${++seq}`;
      effects.set(body.idempotency_key, {
        idempotency_key: body.idempotency_key,
        record: body.record,
        destination_id,
      });
      sendJson(res, 200, { destination_id });
      return;
    }
    sendJson(res, 404, { error: "not_found" });
  } catch (err) {
    process.stderr.write(`control error: ${err.stack || err}\n`);
    if (!res.headersSent) sendJson(res, 500, { error: "internal" });
  }
});

function listen(server) {
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => resolve(server.address().port));
  });
}

const publicPort = await listen(publicServer);
const controlPort = await listen(controlServer);
const ready = { publicPort, controlPort };
fs.writeFileSync(readyPath, JSON.stringify(ready));
process.stdout.write(`${JSON.stringify(ready)}\n`);
