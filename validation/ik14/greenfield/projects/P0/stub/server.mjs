import http from "node:http";
import { createHash } from "node:crypto";

const effects = [];
const byDispatch = new Map();
const byPrinter = new Map();
const notices = [];
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

function publicPrint(row) {
  return {
    dispatch_id: row.dispatch_id,
    printer_id: row.printer_id,
    job_id: row.job_id,
    digest: row.digest,
    outcome: "printed",
    label_b64: row.label_b64,
  };
}

async function onPrint(req, res) {
  let body;
  try {
    body = JSON.parse((await readBody(req, 2_000_000)).toString("utf8"));
  } catch {
    send(res, 400, { error: "bad_request" });
    return;
  }
  const dispatchId = body.dispatch_id;
  const printerId = body.printer_id;
  const jobId = body.job_id;
  const digest = body.digest;
  const labelB64 = body.label_b64;
  if (typeof dispatchId !== "string" || dispatchId.length === 0 ||
      typeof printerId !== "string" || printerId.length === 0 ||
      typeof jobId !== "string" || jobId.length === 0 ||
      typeof digest !== "string" || typeof labelB64 !== "string") {
    send(res, 400, { error: "bad_request" });
    return;
  }
  const raw = Buffer.from(labelB64, "base64");
  if (digestOf(raw) !== digest) {
    send(res, 400, { error: "digest_mismatch" });
    return;
  }
  const prior = byDispatch.get(dispatchId);
  if (prior) {
    if (prior.digest !== digest || prior.label_b64 !== labelB64) {
      send(res, 409, { error: "conflict" });
      return;
    }
  } else {
    const row = {
      dispatch_id: dispatchId,
      printer_id: printerId,
      job_id: jobId,
      digest,
      label_b64: labelB64,
    };
    effects.push(row);
    byDispatch.set(dispatchId, row);
    byPrinter.set(printerId, row);
    notices.push({
      notice_id: "nt-" + dispatchId,
      dispatch_id: dispatchId,
      printer_id: printerId,
      job_id: jobId,
      digest,
      outcome: "printed",
    });
  }
  const row = byDispatch.get(dispatchId);
  const delay = Number(fault.delay_ms) || 0;
  if (delay > 0) {
    await new Promise((r) => setTimeout(r, delay));
  }
  if (fault.drop_reply) {
    req.socket.destroy();
    return;
  }
  send(res, 200, publicPrint(row));
}

function onService(req, res) {
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (req.method === "POST" && url.pathname === "/v1/print") {
    onPrint(req, res).catch(() => {
      if (!res.headersSent) send(res, 500, { error: "internal" });
    });
    return;
  }
  if (req.method === "GET" && url.pathname.startsWith("/v1/dispatches/")) {
    const id = decodeURIComponent(url.pathname.slice("/v1/dispatches/".length));
    const row = byDispatch.get(id);
    if (!row) send(res, 404, { error: "not_found" });
    else send(res, 200, publicPrint(row));
    return;
  }
  if (req.method === "GET" && url.pathname.startsWith("/v1/printers/")) {
    const id = decodeURIComponent(url.pathname.slice("/v1/printers/".length));
    const row = byPrinter.get(id);
    if (!row) send(res, 404, { error: "not_found" });
    else send(res, 200, publicPrint(row));
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
  if (req.method === "GET" && url.pathname === "/notices") {
    send(res, 200, { notices });
    return;
  }
  if (req.method === "POST" && url.pathname === "/fault") {
    readBody(req, 100_000).then((buf) => {
      const patch = JSON.parse(buf.toString("utf8") || "{}");
      if (typeof patch.drop_reply === "boolean") fault.drop_reply = patch.drop_reply;
      if (typeof patch.delay_ms === "number" && patch.delay_ms >= 0) fault.delay_ms = patch.delay_ms;
      send(res, 200, fault);
    }).catch(() => send(res, 400, { error: "bad_request" }));
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
