'use strict';

const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { setTimeout: delay } = require('node:timers/promises');

class Refusal {
  constructor(code) {
    this.code = code;
    this.name = 'Refusal';
  }
}

function reply(obj, code) {
  process.stdout.write(JSON.stringify(obj) + '\n');
  process.exit(code);
}

function defect() {
  return process.env.DEFECT || 'none';
}

function stateDir() {
  if (!process.env.STATE_DIR || !process.env.SERVICE_URL) throw new Refusal('invalid');
  fs.mkdirSync(process.env.STATE_DIR, { recursive: true });
  return process.env.STATE_DIR;
}

function storePath() {
  return path.join(stateDir(), 'jobs.json');
}

function load() {
  const file = storePath();
  if (!fs.existsSync(file)) return { jobs: {} };
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function save(db) {
  const file = storePath();
  const tmp = file + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(db));
  fs.renameSync(tmp, file);
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

async function withLock(fn) {
  const dir = stateDir();
  const lock = path.join(dir, '.lock');
  for (let attempt = 0; attempt < 500; attempt += 1) {
    try {
      fs.mkdirSync(lock);
      try {
        return await fn();
      } finally {
        fs.rmSync(lock, { recursive: true, force: true });
      }
    } catch (err) {
      if (err && err.code === 'EEXIST') {
        await delay(10);
        continue;
      }
      throw err;
    }
  }
  throw new Refusal('unavailable');
}

function validJobId(value) {
  return typeof value === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(value);
}

function validInput(value) {
  return typeof value === 'string' && /^[A-Za-z0-9_.:/-]{1,256}$/.test(value);
}

function submissionId(jobId, revision) {
  return jobId + ':r' + String(revision);
}

function isTerminal(status) {
  return status === 'succeeded' || status === 'failed' || status === 'cancelled';
}

function unapplied(job) {
  return job.status === 'accepted' || job.progress === 'submit_uncertain';
}

function crash(label) {
  if (process.env.CRASH_AT === label) process.kill(process.pid, 'SIGKILL');
}

function serviceUrl(pathnameAndQuery) {
  return process.env.SERVICE_URL.replace(/\/$/, '') + pathnameAndQuery;
}

function requestJson(method, pathnameAndQuery, body, crashBoundaries) {
  const target = new URL(serviceUrl(pathnameAndQuery));
  const payload = body ? JSON.stringify(body) : null;
  return new Promise((resolve) => {
    const req = http.request({
      protocol: target.protocol,
      hostname: target.hostname,
      port: target.port,
      path: target.pathname + target.search,
      method,
      headers: payload ? {
        'content-type': 'application/json',
        'content-length': Buffer.byteLength(payload),
      } : {},
    }, (res) => {
      if (crashBoundaries) crash('after_send');
      const chunks = [];
      res.on('data', (chunk) => chunks.push(chunk));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let parsed = null;
        if (text) {
          try { parsed = JSON.parse(text); } catch { parsed = null; }
        }
        if (crashBoundaries && res.statusCode === 200 && parsed) crash('before_apply_result');
        resolve({ statusCode: res.statusCode, body: parsed, lost: false });
      });
    });
    req.setTimeout(20000, () => req.destroy(new Error('timeout')));
    req.on('error', () => resolve({ lost: true, unavailable: true }));
    if (payload) req.write(payload);
    req.end();
  });
}

async function lookup(submission) {
  return requestJson('GET', '/v1/renders/' + encodeURIComponent(submission), null, false);
}

async function postSubmission(job, crashBoundaries) {
  return requestJson('POST', '/v1/renders', {
    submission_id: job.submission_id,
    job_id: job.job_id,
    input_ref: job.input_ref,
    revision: job.revision,
  }, crashBoundaries);
}

async function postDup(job) {
  const dupId = job.submission_id + ':dup';
  return requestJson('POST', '/v1/renders', {
    submission_id: dupId,
    job_id: job.job_id,
    input_ref: job.input_ref,
    revision: job.revision,
  }, false);
}

async function fetchCallbacks(jobId) {
  const res = await requestJson('GET', '/v1/callbacks?job_id=' + encodeURIComponent(jobId), null, false);
  if (!res.body || !Array.isArray(res.body.events)) return [];
  return res.body.events;
}

function adopt(job, body) {
  if (!body || body.submission_id !== job.submission_id) return false;
  if (body.state === 'succeeded') {
    job.status = 'succeeded';
    job.output_ref = body.output_ref;
    job.progress = 'done';
    return true;
  }
  if (body.state === 'failed') {
    job.status = 'failed';
    job.output_ref = null;
    job.progress = 'failed';
    return true;
  }
  job.status = 'rendering';
  job.output_ref = null;
  job.progress = 'rendering';
  return true;
}

async function readJob(jobId) {
  const db = load();
  const job = db.jobs[jobId];
  if (!job) throw new Refusal('not_found');
  return clone(job);
}

async function markUncertain(jobId, revision) {
  return withLock(async () => {
    const db = load();
    const cur = db.jobs[jobId];
    if (!cur || cur.revision !== revision || isTerminal(cur.status)) return clone(cur);
    cur.status = 'accepted';
    cur.progress = 'submit_uncertain';
    cur.output_ref = null;
    save(db);
    return clone(cur);
  });
}

async function applyCorrect(jobId, revision, submission, body) {
  return withLock(async () => {
    const db = load();
    const cur = db.jobs[jobId];
    if (!cur) return null;
    if (cur.revision !== revision || cur.submission_id !== submission || isTerminal(cur.status)) return clone(cur);
    adopt(cur, body);
    save(db);
    return clone(cur);
  });
}

async function blindSucceed(jobId, outputRef) {
  return withLock(async () => {
    const db = load();
    const cur = db.jobs[jobId];
    cur.status = 'succeeded';
    cur.output_ref = outputRef;
    cur.progress = 'done';
    save(db);
    return clone(cur);
  });
}

async function onLost(job) {
  if (defect() === 'duplicate-effect' || defect() === 'lost-reply-misattribution') {
    await postDup(job);
  }
  if (defect() === 'lost-reply-misattribution') throw new Refusal('not_sent');
  return markUncertain(job.job_id, job.revision);
}

async function applyAfterSend(job, remote) {
  if (remote.lost) return onLost(job);
  if (!remote.body || remote.statusCode !== 200) throw new Refusal('unavailable');
  if (defect() === 'late-overwrite' && remote.body.state === 'succeeded') {
    return blindSucceed(job.job_id, remote.body.output_ref);
  }
  return applyCorrect(job.job_id, job.revision, job.submission_id, remote.body);
}

async function submit(jobId, inputRef) {
  if (!validJobId(jobId) || !validInput(inputRef)) throw new Refusal('invalid');
  const created = await withLock(async () => {
    const db = load();
    if (db.jobs[jobId]) throw new Refusal('already_exists');
    const job = {
      job_id: jobId,
      status: 'accepted',
      input_ref: inputRef,
      revision: 1,
      output_ref: null,
      progress: 'accepted',
      submission_id: submissionId(jobId, 1),
    };
    db.jobs[jobId] = job;
    save(db);
    return clone(job);
  });
  crash('after_accept');
  const remote = await postSubmission(created, true);
  return applyAfterSend(created, remote);
}

async function cancel(jobId) {
  if (!validJobId(jobId)) throw new Refusal('invalid');
  const job = await withLock(async () => {
    const db = load();
    const cur = db.jobs[jobId];
    if (!cur) throw new Refusal('not_found');
    if (cur.status === 'cancelled') return clone(cur);
    if (cur.status === 'succeeded' || cur.status === 'failed') throw new Refusal('terminal');
    cur.status = 'cancelled';
    cur.progress = 'cancelled';
    cur.output_ref = null;
    save(db);
    return clone(cur);
  });
  crash('after_cancel');
  return job;
}

async function status(jobId) {
  if (!validJobId(jobId)) throw new Refusal('invalid');
  return readJob(jobId);
}

async function replace(jobId, inputRef) {
  if (!validJobId(jobId) || !validInput(inputRef)) throw new Refusal('invalid');
  const job = await withLock(async () => {
    const db = load();
    const cur = db.jobs[jobId];
    if (!cur) throw new Refusal('not_found');
    if (isTerminal(cur.status)) throw new Refusal('terminal');
    cur.revision += 1;
    cur.input_ref = inputRef;
    cur.submission_id = submissionId(jobId, cur.revision);
    cur.status = 'accepted';
    cur.progress = 'accepted';
    cur.output_ref = null;
    save(db);
    return clone(cur);
  });
  crash('after_accept');
  const remote = await postSubmission(job, true);
  return applyAfterSend(job, remote);
}

async function overwriteFromCallbacks(job) {
  const events = await fetchCallbacks(job.job_id);
  const foreign = events.filter((event) => event.state === 'succeeded' && event.submission_id !== job.submission_id);
  if (!foreign.length) return null;
  return blindSucceed(job.job_id, foreign[0].output_ref);
}

async function reconcile(jobId) {
  if (!validJobId(jobId)) throw new Refusal('invalid');
  const snap = await readJob(jobId);
  if (defect() === 'late-overwrite') {
    const overwritten = await overwriteFromCallbacks(snap);
    if (overwritten) return overwritten;
    const looked = await lookup(snap.submission_id);
    if (looked.statusCode === 200 && looked.body && looked.body.state === 'succeeded') {
      return blindSucceed(snap.job_id, looked.body.output_ref);
    }
  }
  if (isTerminal(snap.status)) return snap;
  const looked = await lookup(snap.submission_id);
  if (looked.lost || looked.unavailable) throw new Refusal('unavailable');
  if (looked.statusCode === 200 && unapplied(snap)) {
    if (defect() === 'duplicate-effect') {
      const dup = await postDup(snap);
      const output = dup.body && dup.body.output_ref ? dup.body.output_ref : 'dup';
      return blindSucceed(snap.job_id, output);
    }
    if (defect() === 'lost-reply-misattribution') {
      await postDup(snap);
      throw new Refusal('not_sent');
    }
  }
  if (looked.statusCode === 404) {
    const posted = await postSubmission(snap, true);
    return applyAfterSend(snap, posted);
  }
  if (looked.statusCode !== 200 || !looked.body) throw new Refusal('unavailable');
  return applyCorrect(snap.job_id, snap.revision, snap.submission_id, looked.body);
}

async function main() {
  const [cmd, a, b] = process.argv.slice(2);
  let result;
  if (cmd === 'submit') {
    if (process.argv.length !== 5) throw new Refusal('invalid');
    result = await submit(a, b);
  } else if (cmd === 'cancel') {
    if (process.argv.length !== 4) throw new Refusal('invalid');
    result = await cancel(a);
  } else if (cmd === 'status') {
    if (process.argv.length !== 4) throw new Refusal('invalid');
    result = await status(a);
  } else if (cmd === 'reconcile') {
    if (process.argv.length !== 4) throw new Refusal('invalid');
    result = await reconcile(a);
  } else if (cmd === 'replace') {
    if (process.argv.length !== 5) throw new Refusal('invalid');
    result = await replace(a, b);
  } else {
    throw new Refusal('unknown_command');
  }
  reply(result, 0);
}

main().catch((err) => {
  if (err instanceof Refusal) reply({ error: err.code }, 1);
  process.stderr.write(String(err && err.stack ? err.stack : err) + '\n');
  reply({ error: 'internal' }, 1);
});
