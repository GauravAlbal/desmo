import http from "node:http";
import { createHash } from "node:crypto";

const effects = [];
const byRequest = new Map();
const byDevice = new Map();
const callbacks = [];
const fault = { drop_reply: false, delay_ms: 0 };

function send(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

function readBody(req, limit) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let n = 0;
    req.on("data", (c) => {
      n += c.length;
      if (n > limit) {
        reject(new Error("too big"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => resolve(Buffer.concat(chunks)));
    req.on("error", reject);
  });
}

function digestOf(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function publicEffect(row) {
  return {
    request_id: row.request_id,
    device_id: row.device_id,
    digest: row.digest,
    outcome: "installed",
    config_b64: row.config_b64,
  };
}

async function onInstall(req, res) {
  let body;
  try {
    body = JSON.parse((await readBody(req, 2_000_000)).toString("utf8"));
  } catch {
    send(res, 400, { error: "bad_json" });
    return;
  }
  const requestId = body.request_id;
  const deviceId = body.device_id;
  const digest = body.digest;
  const configB64 = body.config_b64;
  if (typeof requestId !== "string" || requestId.length === 0 ||
      typeof deviceId !== "string" || deviceId.length === 0 ||
      typeof digest !== "string" || typeof configB64 !== "string") {
    send(res, 400, { error: "digest_mismatch" });
    return;
  }
  const raw = Buffer.from(configB64, "base64");
  if (digestOf(raw) !== digest) {
    send(res, 400, { error: "digest_mismatch" });
    return;
  }
  const prior = byRequest.get(requestId);
  if (prior) {
    if (prior.digest !== digest || prior.config_b64 !== configB64) {
      send(res, 409, { error: "conflict" });
      return;
    }
  } else {
    const row = { request_id: requestId, device_id: deviceId, digest, config_b64: configB64 };
    effects.push(row);
    byRequest.set(requestId, row);
    byDevice.set(deviceId, row);
    callbacks.push({
      callback_id: "cb-" + requestId,
      request_id: requestId,
      device_id: deviceId,
      digest,
      outcome: "installed",
    });
  }
  const row = byRequest.get(requestId);
  const delay = Number(fault.delay_ms) || 0;
  if (delay > 0) {
    await new Promise((r) => setTimeout(r, delay));
  }
  if (fault.drop_reply) {
    req.socket.destroy();
    return;
  }
  send(res, 200, publicEffect(row));
}

function onService(req, res) {
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (req.method === "POST" && url.pathname === "/v1/install") {
    onInstall(req, res).catch(() => {
      if (!res.headersSent) send(res, 500, { error: "internal" });
    });
    return;
  }
  if (req.method === "GET" && url.pathname.startsWith("/v1/requests/")) {
    const id = decodeURIComponent(url.pathname.slice("/v1/requests/".length));
    const row = byRequest.get(id);
    if (!row) send(res, 404, { error: "not_found" });
    else send(res, 200, publicEffect(row));
    return;
  }
  if (req.method === "GET" && url.pathname.startsWith("/v1/devices/")) {
    const id = decodeURIComponent(url.pathname.slice("/v1/devices/".length));
    const row = byDevice.get(id);
    if (!row) send(res, 404, { error: "not_found" });
    else send(res, 200, publicEffect(row));
    return;
  }
  send(res, 404, { error: "not_found" });
}

function onControl(req, res) {
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (req.method === "GET" && url.pathname === "/health") {
    send(res, 200, { ok: true });
    return;
  }
  if (req.method === "GET" && url.pathname === "/effects") {
    send(res, 200, { count: effects.length, effects });
    return;
  }
  if (req.method === "GET" && url.pathname === "/callbacks") {
    send(res, 200, { callbacks });
    return;
  }
  if (req.method === "POST" && url.pathname === "/fault") {
    readBody(req, 100_000).then((buf) => {
      const patch = JSON.parse(buf.toString("utf8") || "{}");
      if (typeof patch.drop_reply === "boolean") fault.drop_reply = patch.drop_reply;
      if (typeof patch.delay_ms === "number" && patch.delay_ms >= 0) fault.delay_ms = patch.delay_ms;
      send(res, 200, fault);
    }).catch(() => send(res, 400, { error: "bad_json" }));
    return;
  }
  send(res, 404, { error: "not_found" });
}

const service = http.createServer(onService);
const control = http.createServer(onControl);
service.on("clientError", (_err, socket) => socket.destroy());
control.on("clientError", (_err, socket) => socket.destroy());
process.on("SIGTERM", () => process.exit(0));

service.listen(0, "127.0.0.1", () => {
  control.listen(0, "127.0.0.1", () => {
    const s = service.address().port;
    const c = control.address().port;
    process.stdout.write(JSON.stringify({
      service_url: "http://127.0.0.1:" + s,
      control_url: "http://127.0.0.1:" + c,
    }) + "\n");
  });
});
