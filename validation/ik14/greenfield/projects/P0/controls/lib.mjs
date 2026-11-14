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
  return path.join(process.env.STATE_DIR, "label-spool.json");
}

function emptyState() {
  return { printer_id: null, jobs: {} };
}

function load() {
  requireStateDir();
  const file = stateFile();
  if (!existsSync(file)) return emptyState();
  const raw = JSON.parse(readFileSync(file, "utf8"));
  return { printer_id: raw.printer_id || null, jobs: raw.jobs || {} };
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
    refuse("unreadable_label");
  }
  if (buf.length === 0) refuse("empty_label");
  if (buf.length > MAX) refuse("too_large");
  return { b64: buf.toString("base64"), digest: digestOf(buf) };
}

function newId() {
  return randomBytes(16).toString("hex");
}

function jobOf(state, jobId) {
  if (!jobId || !state.jobs[jobId]) refuse("not_found");
  return state.jobs[jobId];
}

function statusView(state, job) {
  return {
    job_id: job.job_id,
    printer_id: state.printer_id,
    digest: job.digest,
    label_b64: job.label_b64,
    state: job.state,
    dispatch_id: job.dispatch_id,
    printed_digest: job.printed_digest,
    reprint_of: job.reprint_of,
  };
}

function printedView(state, job) {
  return {
    job_id: job.job_id,
    printer_id: state.printer_id,
    dispatch_id: job.dispatch_id,
    digest: job.digest,
    state: "printed",
    printed_digest: job.printed_digest,
  };
}

function submit(printerId, file) {
  if (!ID_RE.test(printerId || "")) refuse("bad_printer");
  const state = load();
  if (state.printer_id && state.printer_id !== printerId) refuse("printer_bound");
  const cap = capture(file);
  const jobId = newId();
  state.printer_id = printerId;
  state.jobs[jobId] = {
    job_id: jobId,
    digest: cap.digest,
    label_b64: cap.b64,
    state: "queued",
    dispatch_id: null,
    printed_digest: null,
    reprint_of: null,
  };
  commit(state);
  ok({
    job_id: jobId,
    printer_id: printerId,
    digest: cap.digest,
    state: "queued",
  });
}

function hold(jobId) {
  const state = load();
  const job = jobOf(state, jobId);
  if (job.state !== "queued") refuse("not_queued");
  job.state = "held";
  commit(state);
  ok({ job_id: job.job_id, state: "held" });
}

function cancel(jobId) {
  const state = load();
  const job = jobOf(state, jobId);
  if (job.state !== "queued" && job.state !== "held") refuse("not_cancellable");
  job.state = "cancelled";
  commit(state);
  ok({ job_id: job.job_id, state: "cancelled" });
}

function status(jobId) {
  const state = load();
  ok(statusView(state, jobOf(state, jobId)));
}

function reprint(jobId) {
  const state = load();
  const src = jobOf(state, jobId);
  if (src.state !== "printed") refuse("not_printed");
  const id = newId();
  state.jobs[id] = {
    job_id: id,
    digest: src.digest,
    label_b64: src.label_b64,
    state: "queued",
    dispatch_id: null,
    printed_digest: null,
    reprint_of: src.job_id,
  };
  commit(state);
  ok({
    job_id: id,
    printer_id: state.printer_id,
    digest: src.digest,
    state: "queued",
    reprint_of: src.job_id,
  });
}

async function lookup(dispatchId) {
  const res = await fetch(serviceBase() + "/v1/dispatches/" + encodeURIComponent(dispatchId));
  if (res.status === 404) return null;
  if (!res.ok) refuse("lookup_failed", { dispatch_id: dispatchId });
  return await res.json();
}

async function postPrint(state, job) {
  const res = await fetch(serviceBase() + "/v1/print", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      dispatch_id: job.dispatch_id,
      printer_id: state.printer_id,
      job_id: job.job_id,
      digest: job.digest,
      label_b64: job.label_b64,
    }),
  });
  const text = await res.text();
  if (!res.ok) refuse("print_rejected", { dispatch_id: job.dispatch_id });
  return JSON.parse(text);
}

function adopt(state, job, body) {
  if (!body || body.outcome !== "printed" || body.digest !== job.digest) {
    refuse("digest_mismatch");
  }
  job.state = "printed";
  job.printed_digest = body.digest;
  commit(state);
  crash("after-record-before-exit");
  ok(printedView(state, job));
}

async function dispatch(jobId) {
  const state = load();
  const job = jobOf(state, jobId);
  if (job.state === "printed") ok(printedView(state, job));
  if (job.state === "held" || job.state === "cancelled") refuse("not_dispatchable");
  if (job.state !== "queued" && job.state !== "dispatch_saved") refuse("not_dispatchable");
  const recovering = job.state === "dispatch_saved" && !!job.dispatch_id;
  if (!recovering) {
    job.dispatch_id = newId();
    job.state = "dispatch_saved";
    commit(state);
    crash("after-dispatch-saved-before-request");
  } else if (defect === "duplicate-effect") {
    job.dispatch_id = newId();
    commit(state);
  } else {
    const found = await lookup(job.dispatch_id);
    if (found) adopt(state, job, found);
  }
  try {
    const body = await postPrint(state, job);
    crash("after-accept-before-record");
    adopt(state, job, body);
  } catch (err) {
    if (err instanceof Exit) throw err;
    refuse("no_reply", { job_id: job.job_id, dispatch_id: job.dispatch_id });
  }
}

async function reconcile(jobId) {
  const state = load();
  const job = jobOf(state, jobId);
  if (defect === "lost-reply-misattribution") {
    if (job.dispatch_id) {
      try {
        await lookup(job.dispatch_id);
      } catch (err) {
        if (err instanceof Exit) throw err;
      }
    }
    ok({
      job_id: job.job_id,
      state: "not_sent",
      dispatch_id: job.dispatch_id,
      digest: job.digest,
      printed_digest: null,
    });
  }
  if (job.state === "printed") {
    ok({
      job_id: job.job_id,
      state: "printed",
      dispatch_id: job.dispatch_id,
      digest: job.digest,
      printed_digest: job.printed_digest,
    });
  }
  if (job.state !== "dispatch_saved" || !job.dispatch_id) {
    ok({
      job_id: job.job_id,
      state: job.state,
      dispatch_id: null,
      digest: job.digest,
      printed_digest: null,
    });
  }
  const found = await lookup(job.dispatch_id);
  if (!found) {
    ok({
      job_id: job.job_id,
      state: "not_sent",
      dispatch_id: job.dispatch_id,
      digest: job.digest,
      printed_digest: null,
    });
  }
  adopt(state, job, found);
}

function notice(file) {
  const state = load();
  let note;
  try {
    note = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    refuse("bad_notice");
  }
  for (const field of ["notice_id", "dispatch_id", "printer_id", "job_id", "digest", "outcome"]) {
    if (typeof note[field] !== "string" || note[field].length === 0) refuse("bad_notice");
  }
  if (note.outcome !== "printed" && note.outcome !== "rejected") refuse("bad_notice");
  let matched = null;
  for (const job of Object.values(state.jobs)) {
    if (job.dispatch_id === note.dispatch_id) matched = job;
  }
  if (matched && matched.state === "printed" && defect === "late-overwrite") {
    if (note.outcome === "rejected") {
      matched.state = "rejected";
      matched.printed_digest = null;
    } else {
      matched.state = "printed";
      matched.printed_digest = note.digest;
    }
    commit(state);
    ok({
      applied: true,
      job_id: matched.job_id,
      state: matched.state,
      printed_digest: matched.printed_digest,
    });
  }
  ok({ applied: false });
}

export async function main(kind) {
  defect = kind;
  try {
    if (defect === "always-refuse") refuse("refused");
    const cmd = process.argv[2];
    if (cmd === "submit") submit(process.argv[3], process.argv[4]);
    else if (cmd === "hold") hold(process.argv[3]);
    else if (cmd === "cancel") cancel(process.argv[3]);
    else if (cmd === "dispatch") await dispatch(process.argv[3]);
    else if (cmd === "status") status(process.argv[3]);
    else if (cmd === "reconcile") await reconcile(process.argv[3]);
    else if (cmd === "notice") notice(process.argv[3]);
    else if (cmd === "reprint") reprint(process.argv[3]);
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
