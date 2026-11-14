import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import net from 'node:net';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const PROJECT_DIR = process.env.PROJECT_DIR;
const STUB_DIR = process.env.STUB_DIR;
const INCLUDE_CHANGE = process.env.EVAL_SCOPE !== 'brief';
if (!PROJECT_DIR || !STUB_DIR) throw new Error('PROJECT_DIR and STUB_DIR are required');

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
    server.on('error', reject);
  });
}

function parseStdout(stdout) {
  const text = (stdout || '').trim();
  if (!text) return { json: null, parseError: new Error('empty stdout') };
  try {
    const json = JSON.parse(text);
    if (!json || typeof json !== 'object' || Array.isArray(json)) {
      return { json: null, parseError: new Error('stdout is not an object') };
    }
    return { json, parseError: null };
  } catch (err) {
    return { json: null, parseError: err };
  }
}

function childEnv(state, serviceUrl, extra) {
  const env = {
    PATH: process.env.PATH,
    HOME: process.env.HOME || '',
    TMPDIR: process.env.TMPDIR || '/tmp',
    LANG: process.env.LANG || 'C',
    STATE_DIR: state,
    SERVICE_URL: serviceUrl,
  };
  if (extra && extra.CRASH_AT) env.CRASH_AT = extra.CRASH_AT;
  return env;
}

function run(fx, args, extra) {
  const result = spawnSync(path.join(PROJECT_DIR, 'run'), args, {
    env: childEnv(fx.state, fx.serviceUrl, extra),
    encoding: 'utf8',
    timeout: 20000,
  });
  return { status: result.status, signal: result.signal, stdout: result.stdout, stderr: result.stderr, ...parseStdout(result.stdout) };
}

function mustOk(result, label) {
  assert.equal(result.signal, null, label + ' signal ' + result.signal + ' stderr ' + result.stderr);
  assert.equal(result.status, 0, label + ' exit ' + result.status + ' stdout ' + result.stdout + ' stderr ' + result.stderr);
  assert.equal(result.parseError, null, label + ' parse ' + result.stdout);
  assert.ok(result.json && !result.json.error, label + ' body ' + JSON.stringify(result.json));
  return result.json;
}

function assertFields(actual, expected) {
  for (const [key, value] of Object.entries(expected)) {
    assert.deepEqual(actual[key], value, key + ' got ' + JSON.stringify(actual[key]) + ' full ' + JSON.stringify(actual));
  }
}

async function fault(controlUrl, body) {
  const res = await fetch(controlUrl + '/fault', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error('fault ' + res.status + ' ' + await res.text());
  return res.json();
}

async function effects(controlUrl) {
  const res = await fetch(controlUrl + '/effects');
  if (!res.ok) throw new Error('effects ' + res.status);
  return res.json();
}

function idsFor(body, jobId) {
  return body.items.filter((item) => item.job_id === jobId).map((item) => item.submission_id).sort();
}

async function start(t) {
  const port = await freePort();
  const controlPort = await freePort();
  const state = mkdtempSync(path.join(tmpdir(), 'g1-'));
  const child = spawn(process.execPath, [path.join(STUB_DIR, 'server.mjs')], {
    env: { ...process.env, PORT: String(port), CONTROL_PORT: String(controlPort) },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let logs = '';
  child.stdout.on('data', (chunk) => { logs += chunk; });
  child.stderr.on('data', (chunk) => { logs += chunk; });
  const serviceUrl = 'http://127.0.0.1:' + port;
  const controlUrl = 'http://127.0.0.1:' + controlPort;
  t.after(() => {
    if (child.exitCode == null && child.signalCode == null) child.kill('SIGTERM');
    rmSync(state, { recursive: true, force: true });
  });
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (child.exitCode != null) throw new Error('stub exited ' + logs);
    try {
      const res = await fetch(controlUrl + '/health');
      if (res.ok) return { state, serviceUrl, controlUrl, child, logs: () => logs };
    } catch { /* not up yet */ }
    await delay(20);
  }
  throw new Error('stub not ready ' + logs);
}

async function stopStub(fx) {
  if (fx.child.exitCode != null || fx.child.signalCode != null) return;
  fx.child.kill('SIGTERM');
  await new Promise((resolve) => fx.child.once('exit', resolve));
}

function out(submission) {
  return 'out:' + submission;
}

test('R1 submit accepts a new job and rejects a duplicate id', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  const created = mustOk(run(fx, ['submit', 'job1', 'media/clip-a']), 'submit');
  assertFields(created, {
    job_id: 'job1',
    input_ref: 'media/clip-a',
    revision: 1,
    submission_id: 'job1:r1',
    status: 'rendering',
    output_ref: null,
    progress: 'rendering',
  });
  const admitted = await effects(fx.controlUrl);
  assert.deepEqual(idsFor(admitted, 'job1'), ['job1:r1']);
  const dup = run(fx, ['submit', 'job1', 'media/other']);
  assert.notEqual(dup.status, 0);
  assert.equal(dup.json && dup.json.error, 'already_exists');
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1']);
});

test('R2 status reads the durable record after restart with the worker stopped', async (t) => {
  const fx = await start(t);
  const created = mustOk(run(fx, ['submit', 'job1', 'media/x']), 'submit');
  assertFields(created, { status: 'succeeded', output_ref: out('job1:r1'), progress: 'done', revision: 1 });
  await stopStub(fx);
  const later = mustOk(run(fx, ['status', 'job1']), 'status');
  assertFields(later, {
    job_id: 'job1',
    input_ref: 'media/x',
    revision: 1,
    submission_id: 'job1:r1',
    status: 'succeeded',
    output_ref: out('job1:r1'),
    progress: 'done',
  });
});

test('R3 crash after accept keeps the job and reconcile admits it once', async (t) => {
  const fx = await start(t);
  const crashed = run(fx, ['submit', 'job1', 'media/x'], { CRASH_AT: 'after_accept' });
  assert.equal(crashed.signal, 'SIGKILL');
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), []);
  const stored = mustOk(run(fx, ['status', 'job1']), 'status');
  assertFields(stored, {
    status: 'accepted',
    progress: 'accepted',
    output_ref: null,
    submission_id: 'job1:r1',
    revision: 1,
    input_ref: 'media/x',
  });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'succeeded', output_ref: out('job1:r1'), progress: 'done', submission_id: 'job1:r1' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1']);
});

test('R4 lost reply is not reported as not-sent and is recovered once', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { drop_replies: 1 });
  const dropped = mustOk(run(fx, ['submit', 'dropjob', 'media/d']), 'dropped submit');
  assertFields(dropped, {
    status: 'accepted',
    progress: 'submit_uncertain',
    output_ref: null,
    submission_id: 'dropjob:r1',
    revision: 1,
  });
  assert.notEqual(dropped.error, 'not_sent');
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'dropjob'), ['dropjob:r1']);
  const recovered = mustOk(run(fx, ['reconcile', 'dropjob']), 'reconcile drop');
  assertFields(recovered, { status: 'succeeded', output_ref: out('dropjob:r1'), progress: 'done' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'dropjob'), ['dropjob:r1']);

  const crashed = run(fx, ['submit', 'crashjob', 'media/c'], { CRASH_AT: 'after_send' });
  assert.equal(crashed.signal, 'SIGKILL');
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'crashjob'), ['crashjob:r1']);
  const stored = mustOk(run(fx, ['status', 'crashjob']), 'status crash');
  assert.notEqual(stored.status, 'succeeded');
  assert.equal(stored.output_ref, null);
  assert.equal(stored.submission_id, 'crashjob:r1');
  const again = mustOk(run(fx, ['reconcile', 'crashjob']), 'reconcile crash');
  assertFields(again, { status: 'succeeded', output_ref: out('crashjob:r1'), submission_id: 'crashjob:r1' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'crashjob'), ['crashjob:r1']);
});

test('R5 cancel is durable, idempotent, and visible after after_cancel', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'media/j']), 'submit');
  const cancelled = mustOk(run(fx, ['cancel', 'job1']), 'cancel');
  assertFields(cancelled, { status: 'cancelled', progress: 'cancelled', output_ref: null, revision: 1 });
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { status: 'cancelled', output_ref: null });
  assertFields(mustOk(run(fx, ['cancel', 'job1']), 'cancel again'), { status: 'cancelled', output_ref: null });
  mustOk(run(fx, ['submit', 'job2', 'media/k']), 'submit 2');
  const crashed = run(fx, ['cancel', 'job2'], { CRASH_AT: 'after_cancel' });
  assert.equal(crashed.signal, 'SIGKILL');
  assertFields(mustOk(run(fx, ['status', 'job2']), 'status 2'), { status: 'cancelled', output_ref: null, progress: 'cancelled' });
});

test('R6 a late worker success does not revive a cancelled job', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'media/j']), 'submit');
  mustOk(run(fx, ['cancel', 'job1']), 'cancel');
  await fault(fx.controlUrl, { complete: 'job1:r1' });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'cancelled', output_ref: null });
  assert.notEqual(recovered.status, 'succeeded');
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { status: 'cancelled', output_ref: null });
});

test('R7 a conflicting later report does not replace a committed output', async (t) => {
  const fx = await start(t);
  const created = mustOk(run(fx, ['submit', 'job1', 'media/j']), 'submit');
  assertFields(created, { status: 'succeeded', output_ref: out('job1:r1') });
  await fault(fx.controlUrl, { conflict: { submission_id: 'job1:r1', output_ref: 'late:job1:r1' } });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'succeeded', output_ref: out('job1:r1') });
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { status: 'succeeded', output_ref: out('job1:r1') });
});

test('R8 crash before apply does not commit the result and reconcile applies it once', async (t) => {
  const fx = await start(t);
  const crashed = run(fx, ['submit', 'job1', 'media/j'], { CRASH_AT: 'before_apply_result' });
  assert.equal(crashed.signal, 'SIGKILL');
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1']);
  const stored = mustOk(run(fx, ['status', 'job1']), 'status');
  assert.notEqual(stored.status, 'succeeded');
  assert.equal(stored.output_ref, null);
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'succeeded', output_ref: out('job1:r1'), submission_id: 'job1:r1' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1']);
});

test('R9 an independent job finishes while another is cancelled or only accepted', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'stuck', 'media/stuck']), 'submit stuck');
  mustOk(run(fx, ['cancel', 'stuck']), 'cancel stuck');
  await fault(fx.controlUrl, { hold: false });
  const other = mustOk(run(fx, ['submit', 'other', 'media/other']), 'submit other');
  assertFields(other, { status: 'succeeded', output_ref: out('other:r1'), job_id: 'other' });
  assertFields(mustOk(run(fx, ['status', 'stuck']), 'status stuck'), { status: 'cancelled', output_ref: null });
  const crashed = run(fx, ['submit', 'unsent', 'media/unsent'], { CRASH_AT: 'after_accept' });
  assert.equal(crashed.signal, 'SIGKILL');
  const finished = mustOk(run(fx, ['submit', 'fresh', 'media/fresh']), 'submit fresh');
  assertFields(finished, { status: 'succeeded', output_ref: out('fresh:r1') });
  assertFields(mustOk(run(fx, ['status', 'unsent']), 'status unsent'), { status: 'accepted', output_ref: null, progress: 'accepted' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'other'), ['other:r1']);
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'fresh'), ['fresh:r1']);
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'unsent'), []);
});

test('R10 a healthy submission reaches succeeded and is not admitted twice', async (t) => {
  const fx = await start(t);
  const created = mustOk(run(fx, ['submit', 'job1', 'media/clip']), 'submit');
  assertFields(created, { status: 'succeeded', progress: 'done', output_ref: out('job1:r1'), revision: 1 });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'succeeded', output_ref: out('job1:r1') });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1']);
});

test('R11 a concurrent cancel wins over an in-flight success reply', async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { delay_ms: 4000, delay_job_id: 'slow' });
  const child = spawn(path.join(PROJECT_DIR, 'run'), ['submit', 'slow', 'media/slow'], {
    env: childEnv(fx.state, fx.serviceUrl),
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  const deadline = Date.now() + 8000;
  let seen = false;
  while (Date.now() < deadline) {
    const body = await effects(fx.controlUrl);
    if (idsFor(body, 'slow').includes('slow:r1')) { seen = true; break; }
    await delay(20);
  }
  assert.equal(seen, true, 'submission was not admitted while submit was in flight');
  const cancelled = mustOk(run(fx, ['cancel', 'slow']), 'cancel');
  assertFields(cancelled, { status: 'cancelled', output_ref: null });
  const closed = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('submit did not finish')), 12000);
    child.on('close', (status, signal) => {
      clearTimeout(timer);
      resolve({ status, signal, stdout });
    });
  });
  assert.equal(closed.signal, null, 'submit crashed ' + closed.signal + ' ' + stdout);
  const stored = mustOk(run(fx, ['status', 'slow']), 'status');
  assertFields(stored, { status: 'cancelled', output_ref: null });
  assert.notEqual(stored.status, 'succeeded');
});

test('R12 refusals use the stated error codes and do not clobber success', async (t) => {
  const fx = await start(t);
  const missing = run(fx, ['status', 'nope']);
  assert.notEqual(missing.status, 0);
  assert.equal(missing.json && missing.json.error, 'not_found');
  const missingCancel = run(fx, ['cancel', 'nope']);
  assert.notEqual(missingCancel.status, 0);
  assert.equal(missingCancel.json && missingCancel.json.error, 'not_found');
  const missingReconcile = run(fx, ['reconcile', 'nope']);
  assert.notEqual(missingReconcile.status, 0);
  assert.equal(missingReconcile.json && missingReconcile.json.error, 'not_found');
  const bad = run(fx, ['submit', 'bad id', 'media/x']);
  assert.notEqual(bad.status, 0);
  assert.equal(bad.json && bad.json.error, 'invalid');
  const unknown = run(fx, ['frob']);
  assert.notEqual(unknown.status, 0);
  assert.equal(unknown.json && unknown.json.error, 'unknown_command');
  const created = mustOk(run(fx, ['submit', 'job1', 'media/x']), 'submit');
  assert.equal(created.status, 'succeeded');
  const terminal = run(fx, ['cancel', 'job1']);
  assert.notEqual(terminal.status, 0);
  assert.equal(terminal.json && terminal.json.error, 'terminal');
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { status: 'succeeded', output_ref: out('job1:r1') });
});

test('C1 an old completion does not attach output to the replacement revision', { skip: !INCLUDE_CHANGE }, async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'clip/old']), 'submit');
  const replaced = mustOk(run(fx, ['replace', 'job1', 'clip/new']), 'replace');
  assertFields(replaced, {
    revision: 2,
    input_ref: 'clip/new',
    submission_id: 'job1:r2',
    output_ref: null,
    status: 'rendering',
  });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1', 'job1:r2']);
  await fault(fx.controlUrl, { complete: 'job1:r1' });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { revision: 2, input_ref: 'clip/new', submission_id: 'job1:r2', output_ref: null });
  assert.notEqual(recovered.status, 'succeeded');
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { revision: 2, output_ref: null, input_ref: 'clip/new' });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1', 'job1:r2']);
});

test('C2 the replacement revision can still complete', { skip: !INCLUDE_CHANGE }, async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'clip/old']), 'submit');
  mustOk(run(fx, ['replace', 'job1', 'clip/new']), 'replace');
  await fault(fx.controlUrl, { complete: 'job1:r2' });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, {
    status: 'succeeded',
    output_ref: out('job1:r2'),
    revision: 2,
    input_ref: 'clip/new',
    submission_id: 'job1:r2',
    progress: 'done',
  });
});

test('C3 cancel after replace still wins over old and new completions', { skip: !INCLUDE_CHANGE }, async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'clip/old']), 'submit');
  mustOk(run(fx, ['replace', 'job1', 'clip/new']), 'replace');
  assertFields(mustOk(run(fx, ['cancel', 'job1']), 'cancel'), { status: 'cancelled', output_ref: null, revision: 2, input_ref: 'clip/new' });
  const rejected = run(fx, ['replace', 'job1', 'clip/again']);
  assert.notEqual(rejected.status, 0);
  assert.equal(rejected.json && rejected.json.error, 'terminal');
  await fault(fx.controlUrl, { complete: 'job1:r1' });
  await fault(fx.controlUrl, { complete: 'job1:r2' });
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, { status: 'cancelled', output_ref: null, revision: 2, input_ref: 'clip/new' });
  assertFields(mustOk(run(fx, ['status', 'job1']), 'status'), { status: 'cancelled', output_ref: null, revision: 2 });
});

test('C4 a lost reply on the replacement is recovered without a second admission', { skip: !INCLUDE_CHANGE }, async (t) => {
  const fx = await start(t);
  await fault(fx.controlUrl, { hold: true });
  mustOk(run(fx, ['submit', 'job1', 'clip/old']), 'submit');
  await fault(fx.controlUrl, { hold: false, drop_replies: 1 });
  const replaced = mustOk(run(fx, ['replace', 'job1', 'clip/new']), 'replace');
  assertFields(replaced, {
    revision: 2,
    input_ref: 'clip/new',
    submission_id: 'job1:r2',
    progress: 'submit_uncertain',
    output_ref: null,
    status: 'accepted',
  });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1', 'job1:r2']);
  const recovered = mustOk(run(fx, ['reconcile', 'job1']), 'reconcile');
  assertFields(recovered, {
    status: 'succeeded',
    output_ref: out('job1:r2'),
    revision: 2,
    input_ref: 'clip/new',
    submission_id: 'job1:r2',
  });
  assert.deepEqual(idsFor(await effects(fx.controlUrl), 'job1'), ['job1:r1', 'job1:r2']);
});
