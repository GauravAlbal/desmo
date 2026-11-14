/**
 * Fake media-render worker. Public API on PORT. Evaluator control on CONTROL_PORT.
 * State is in memory. Admissions are counted once per distinct submission_id.
 */
import http from 'node:http';

const port = Number(process.env.PORT);
const controlPort = process.env.CONTROL_PORT ? Number(process.env.CONTROL_PORT) : 0;
if (!Number.isInteger(port) || port <= 0) {
  console.error('PORT is required');
  process.exit(1);
}

const state = {
  dropReplies: 0,
  delayMs: 0,
  delayJobId: null,
  hold: false,
  seq: 0,
  admissions: [],
  records: new Map(),
  callbacks: [],
};

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function view(rec) {
  return {
    submission_id: rec.submission_id,
    job_id: rec.job_id,
    input_ref: rec.input_ref,
    revision: rec.revision,
    state: rec.state,
    output_ref: rec.output_ref,
    progress: rec.progress,
  };
}

function pushCallback(rec) {
  state.seq += 1;
  state.callbacks.push({
    event_id: `evt-${state.seq}`,
    submission_id: rec.submission_id,
    job_id: rec.job_id,
    revision: rec.revision,
    state: rec.state,
    output_ref: rec.output_ref,
    seq: state.seq,
  });
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (chunk) => chunks.push(chunk));
    req.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      if (!text) return resolve(null);
      try {
        resolve(JSON.parse(text));
      } catch (err) {
        reject(err);
      }
    });
    req.on('error', reject);
  });
}

function sendJson(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(body),
  });
  res.end(body);
}

function shouldDelay(jobId) {
  if (!state.delayMs) return 0;
  if (state.delayJobId && state.delayJobId !== jobId) return 0;
  return state.delayMs;
}

async function respondPost(req, res, rec) {
  const delay = shouldDelay(rec.job_id);
  if (delay) await sleep(delay);
  if (state.dropReplies > 0) {
    state.dropReplies -= 1;
    req.socket.destroy();
    return;
  }
  sendJson(res, 200, view(rec));
}

async function handlePublic(req, res) {
  const url = new URL(req.url || '/', 'http://127.0.0.1');
  if (req.method === 'GET' && url.pathname === '/v1/callbacks') {
    const jobId = url.searchParams.get('job_id');
    const events = state.callbacks.filter((event) => !jobId || event.job_id === jobId);
    sendJson(res, 200, { events });
    return;
  }
  if (req.method === 'GET' && url.pathname.startsWith('/v1/renders/')) {
    const id = decodeURIComponent(url.pathname.slice('/v1/renders/'.length));
    const rec = state.records.get(id);
    if (!rec) {
      sendJson(res, 404, { error: 'not_found' });
      return;
    }
    sendJson(res, 200, view(rec));
    return;
  }
  if (req.method === 'POST' && url.pathname === '/v1/renders') {
    let body;
    try {
      body = await readBody(req);
    } catch {
      sendJson(res, 400, { error: 'invalid' });
      return;
    }
    if (!body || typeof body.submission_id !== 'string' || typeof body.job_id !== 'string' || typeof body.input_ref !== 'string' || typeof body.revision !== 'number') {
      sendJson(res, 400, { error: 'invalid' });
      return;
    }
    const existing = state.records.get(body.submission_id);
    if (existing) {
      await respondPost(req, res, existing);
      return;
    }
    const rec = {
      submission_id: body.submission_id,
      job_id: body.job_id,
      input_ref: body.input_ref,
      revision: body.revision,
      state: state.hold ? 'rendering' : 'succeeded',
      output_ref: state.hold ? null : `out:${body.submission_id}`,
      progress: state.hold ? 'rendering' : 'done',
    };
    state.records.set(rec.submission_id, rec);
    state.admissions.push({
      submission_id: rec.submission_id,
      job_id: rec.job_id,
      input_ref: rec.input_ref,
      revision: rec.revision,
    });
    pushCallback(rec);
    await respondPost(req, res, rec);
    return;
  }
  sendJson(res, 404, { error: 'not_found' });
}

function complete(id) {
  const rec = state.records.get(id);
  if (!rec) return null;
  if (rec.state !== 'succeeded') {
    rec.state = 'succeeded';
    rec.output_ref = `out:${id}`;
    rec.progress = 'done';
  }
  pushCallback(rec);
  return rec;
}

function conflict(id, outputRef) {
  const rec = state.records.get(id);
  if (!rec) return null;
  rec.state = 'succeeded';
  rec.output_ref = outputRef;
  rec.progress = 'done';
  pushCallback(rec);
  return rec;
}

async function handleControl(req, res) {
  const url = new URL(req.url || '/', 'http://127.0.0.1');
  if (req.method === 'GET' && url.pathname === '/health') {
    sendJson(res, 200, { ok: true });
    return;
  }
  if (req.method === 'GET' && url.pathname === '/effects') {
    sendJson(res, 200, { count: state.admissions.length, items: state.admissions });
    return;
  }
  if (req.method === 'GET' && url.pathname === '/callbacks') {
    const jobId = url.searchParams.get('job_id');
    const events = state.callbacks.filter((event) => !jobId || event.job_id === jobId);
    sendJson(res, 200, { events });
    return;
  }
  if (req.method === 'POST' && url.pathname === '/fault') {
    let body;
    try {
      body = await readBody(req);
    } catch {
      sendJson(res, 400, { error: 'invalid' });
      return;
    }
    body = body || {};
    if (typeof body.drop_replies === 'number') state.dropReplies = body.drop_replies;
    if (typeof body.delay_ms === 'number') state.delayMs = body.delay_ms;
    if (Object.prototype.hasOwnProperty.call(body, 'delay_job_id')) state.delayJobId = body.delay_job_id;
    if (typeof body.hold === 'boolean') state.hold = body.hold;
    if (typeof body.complete === 'string') {
      const rec = complete(body.complete);
      if (!rec) {
        sendJson(res, 404, { error: 'not_found' });
        return;
      }
    }
    if (body.conflict && typeof body.conflict.submission_id === 'string') {
      const rec = conflict(body.conflict.submission_id, body.conflict.output_ref);
      if (!rec) {
        sendJson(res, 404, { error: 'not_found' });
        return;
      }
    }
    sendJson(res, 200, { ok: true, hold: state.hold, drop_replies: state.dropReplies, delay_ms: state.delayMs });
    return;
  }
  sendJson(res, 404, { error: 'not_found' });
}

const publicServer = http.createServer((req, res) => {
  handlePublic(req, res).catch((err) => {
    console.error(err);
    if (!res.headersSent) sendJson(res, 500, { error: 'internal' });
  });
});

function ready() {
  process.stdout.write('READY\n');
}

publicServer.listen(port, '127.0.0.1', () => {
  if (!controlPort) {
    ready();
    return;
  }
  const controlServer = http.createServer((req, res) => {
    handleControl(req, res).catch((err) => {
      console.error(err);
      if (!res.headersSent) sendJson(res, 500, { error: 'internal' });
    });
  });
  controlServer.listen(controlPort, '127.0.0.1', ready);
  process.on('SIGTERM', () => {
    publicServer.close();
    controlServer.close();
    process.exit(0);
  });
});

process.on('SIGTERM', () => {
  publicServer.close();
  process.exit(0);
});
