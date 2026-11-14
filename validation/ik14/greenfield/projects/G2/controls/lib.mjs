import { createHash, randomBytes } from "node:crypto";
import { closeSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeSync, existsSync } from "node:fs";
import path from "node:path";

const MAX = 1024 * 1024;
const ID_RE = /^[A-Za-z0-9._-]{1,64}$/;

class Exit extends Error {}

let defect = "none";

function finish(code, obj) {
  process.stdout.write(JSON.stringify(obj) + "\n", () => process.exit(code));
  throw new Exit();
}

function ok(obj) {
  finish(0, obj);
}

function refuse(error, extra) {
  finish(1, Object.assign({ error }, extra || {}));
}

function crash(label) {
  if (process.env.CRASH_AT === label) {
    process.kill(process.pid, "SIGKILL");
  }
}

function requireStateDir() {
  if (!process.env.STATE_DIR) refuse("missing_state_dir");
  mkdirSync(process.env.STATE_DIR, { recursive: true });
}

function serviceBase() {
  const raw = process.env.SERVICE_URL;
  if (!raw) refuse("missing_service_url");
  return raw.endsWith("/") ? raw.slice(0, -1) : raw;
}

function stateFile() {
  return path.join(process.env.STATE_DIR, "rollout-desk.json");
}

function emptyState() {
  return {
    device_id: null,
    proposal_id: null,
    digest: null,
    bytes_b64: null,
    approval: "none",
    approved_digest: null,
    request_id: null,
    install: "none",
    installed_digest: null,
    shadow_approved: false,
  };
}

function load() {
  requireStateDir();
  const file = stateFile();
  if (!existsSync(file)) return emptyState();
  return Object.assign(emptyState(), JSON.parse(readFileSync(file, "utf8")));
}

function commit(state) {
  requireStateDir();
  const dest = stateFile();
  const tmp = dest + ".tmp";
  const fd = openSync(tmp, "w");
  writeSync(fd, JSON.stringify(state));
  fsyncSync(fd);
  closeSync(fd);
  renameSync(tmp, dest);
  try {
    const dirfd = openSync(process.env.STATE_DIR, "r");
    try {
      fsyncSync(dirfd);
    } finally {
      closeSync(dirfd);
    }
  } catch {
    /* directory fsync is best-effort */
  }
}

function digestOf(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function capture(file) {
  let buf;
  try {
    buf = readFileSync(file);
  } catch {
    refuse("unreadable_config");
  }
  if (buf.length === 0) refuse("empty_config");
  if (buf.length > MAX) refuse("too_large");
  return { b64: buf.toString("base64"), digest: digestOf(buf) };
}

function newId() {
  return randomBytes(16).toString("hex");
}

function statusView(state) {
  return {
    device_id: state.device_id,
    proposal_id: state.proposal_id,
    digest: state.digest,
    approval: state.approval,
    approved_digest: state.approved_digest,
    request_id: state.request_id,
    install: state.install,
    installed_digest: state.installed_digest,
  };
}

function installedView(state) {
  return {
    proposal_id: state.proposal_id,
    device_id: state.device_id,
    request_id: state.request_id,
    digest: state.digest,
    install: "installed",
    installed_digest: state.installed_digest,
  };
}

function authorized(state) {
  if (state.approval === "approved" && state.approved_digest === state.digest) return true;
  if (defect === "stale-approval" && state.shadow_approved) return true;
  return false;
}

function prepare(deviceId, file) {
  if (!ID_RE.test(deviceId || "")) refuse("bad_device");
  const state = load();
  if (state.device_id && state.device_id !== deviceId) refuse("device_bound");
  if (state.proposal_id) refuse("proposal_exists");
  const cap = capture(file);
  state.device_id = deviceId;
  state.proposal_id = newId();
  state.bytes_b64 = cap.b64;
  state.digest = cap.digest;
  state.approval = "none";
  state.approved_digest = null;
  state.install = "none";
  state.request_id = null;
  state.installed_digest = null;
  state.shadow_approved = false;
  commit(state);
  ok({
    proposal_id: state.proposal_id,
    device_id: state.device_id,
    digest: state.digest,
    approval: "none",
    install: "none",
  });
}

function show() {
  const state = load();
  if (!state.proposal_id) refuse("not_found");
  ok({
    proposal_id: state.proposal_id,
    device_id: state.device_id,
    digest: state.digest,
    bytes_b64: state.bytes_b64,
    approval: state.approval,
    approved_digest: state.approved_digest,
    install: state.install,
  });
}

function approve(proposalId, digest) {
  const state = load();
  if (!state.proposal_id || state.proposal_id !== proposalId) refuse("not_found");
  if (digest !== state.digest) refuse("digest_mismatch");
  state.approval = "approved";
  state.approved_digest = digest;
  commit(state);
  ok({
    proposal_id: state.proposal_id,
    digest: state.digest,
    approval: "approved",
    approved_digest: state.approved_digest,
  });
}

function replace(proposalId, file) {
  const state = load();
  if (!state.proposal_id || state.proposal_id !== proposalId) refuse("not_found");
  if (state.install !== "none") refuse("install_started");
  const cap = capture(file);
  if (defect === "stale-approval" && state.approval === "approved") {
    state.shadow_approved = true;
  }
  state.bytes_b64 = cap.b64;
  state.digest = cap.digest;
  state.approval = "none";
  state.approved_digest = null;
  commit(state);
  ok({
    proposal_id: state.proposal_id,
    device_id: state.device_id,
    digest: state.digest,
    approval: "none",
    approved_digest: null,
    install: state.install,
  });
}

function status() {
  ok(statusView(load()));
}

async function lookup(requestId) {
  const res = await fetch(serviceBase() + "/v1/requests/" + encodeURIComponent(requestId));
  if (res.status === 404) return null;
  if (!res.ok) refuse("lookup_failed", { request_id: requestId });
  return await res.json();
}

async function postInstall(state) {
  const res = await fetch(serviceBase() + "/v1/install", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      request_id: state.request_id,
      device_id: state.device_id,
      digest: state.digest,
      config_b64: state.bytes_b64,
    }),
  });
  const text = await res.text();
  if (!res.ok) refuse("install_rejected", { request_id: state.request_id });
  return JSON.parse(text);
}

function adopt(state, body) {
  if (!body || body.outcome !== "installed" || body.digest !== state.digest) {
    refuse("digest_mismatch");
  }
  state.install = "installed";
  state.installed_digest = body.digest;
  commit(state);
  crash("after-record-before-exit");
  ok(installedView(state));
}

async function install() {
  const state = load();
  if (state.install === "installed") ok(installedView(state));
  if (!state.proposal_id || !authorized(state)) refuse("not_approved");
  const recovering = state.install === "attempt_saved" && !!state.request_id;
  if (!recovering) {
    state.request_id = newId();
    state.install = "attempt_saved";
    commit(state);
    crash("after-attempt-saved-before-request");
  } else if (defect === "duplicate-effect") {
    state.request_id = newId();
    commit(state);
  } else {
    const found = await lookup(state.request_id);
    if (found) adopt(state, found);
  }
  try {
    const body = await postInstall(state);
    crash("after-accept-before-record");
    adopt(state, body);
  } catch (err) {
    if (err instanceof Exit) throw err;
    refuse("no_reply", { request_id: state.request_id });
  }
}

async function reconcile() {
  const state = load();
  if (defect === "lost-reply-misattribution") {
    if (state.request_id) {
      try {
        await lookup(state.request_id);
      } catch (err) {
        if (err instanceof Exit) throw err;
      }
    }
    ok({
      install: "not_sent",
      request_id: state.request_id,
      digest: state.digest,
      installed_digest: null,
    });
  }
  if (state.install === "installed" || state.install === "rejected") {
    ok({
      install: state.install,
      request_id: state.request_id,
      digest: state.digest,
      installed_digest: state.installed_digest,
    });
  }
  if (state.install !== "attempt_saved" || !state.request_id) {
    ok({
      install: "none",
      request_id: null,
      digest: state.digest,
      installed_digest: null,
    });
  }
  const found = await lookup(state.request_id);
  if (!found) {
    ok({
      install: "not_sent",
      request_id: state.request_id,
      digest: state.digest,
      installed_digest: null,
    });
  }
  adopt(state, found);
}

function callback(file) {
  const state = load();
  let cb;
  try {
    cb = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    refuse("bad_callback");
  }
  for (const field of ["callback_id", "request_id", "device_id", "digest", "outcome"]) {
    if (typeof cb[field] !== "string" || cb[field].length === 0) refuse("bad_callback");
  }
  if (cb.outcome !== "installed" && cb.outcome !== "rejected") refuse("bad_callback");
  const terminal = state.install === "installed" || state.install === "rejected";
  if (terminal && defect === "late-overwrite") {
    if (cb.outcome === "rejected") {
      state.install = "rejected";
      state.installed_digest = null;
    } else {
      state.install = "installed";
      state.installed_digest = cb.digest;
    }
    commit(state);
    ok({
      applied: true,
      install: state.install,
      installed_digest: state.installed_digest,
      request_id: state.request_id,
    });
  }
  ok({
    applied: false,
    install: state.install,
    installed_digest: state.installed_digest,
    request_id: state.request_id,
  });
}

export async function main(kind) {
  defect = kind;
  try {
    if (defect === "always-refuse") refuse("refused");
    const cmd = process.argv[2];
    if (cmd === "prepare") prepare(process.argv[3], process.argv[4]);
    else if (cmd === "show") show();
    else if (cmd === "approve") approve(process.argv[3], process.argv[4]);
    else if (cmd === "install") await install();
    else if (cmd === "status") status();
    else if (cmd === "reconcile") await reconcile();
    else if (cmd === "callback") callback(process.argv[3]);
    else if (cmd === "replace") replace(process.argv[3], process.argv[4]);
    else refuse("unknown_command");
  } catch (err) {
    if (err instanceof Exit) return;
    console.error(err);
    try {
      refuse("internal");
    } catch {
      /* exit already scheduled */
    }
  }
}
