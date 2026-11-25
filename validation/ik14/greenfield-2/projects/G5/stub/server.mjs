import http from 'node:http';
const port = Number(process.env.PORT || 0);
const tasks = new Map();
const server = http.createServer(async (req, res) => {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const body = Buffer.concat(chunks).toString();
  const send = (status, value) => { res.writeHead(status, {'content-type':'application/json'}); res.end(JSON.stringify(value)); };
  if (req.method === 'POST' && req.url === '/tasks') {
    let task;
    try { task = JSON.parse(body); } catch { return send(400, {error:'invalid JSON'}); }
    if (typeof task.task_id !== 'string' || !task.task_id) return send(400, {error:'task_id required'});
    if (!tasks.has(task.task_id)) tasks.set(task.task_id, {task_id:task.task_id, outcome:'confirmed'});
    return send(200, tasks.get(task.task_id));
  }
  if (req.method === 'GET' && req.url.startsWith('/tasks/')) {
    const id = decodeURIComponent(req.url.slice('/tasks/'.length));
    return tasks.has(id) ? send(200, tasks.get(id)) : send(404, {error:'not found'});
  }
  return send(404, {error:'not found'});
});
server.listen(port, '127.0.0.1', () => process.stdout.write('READY '+server.address().port+'\n'));
process.on('SIGTERM', () => server.close(() => process.exit(0)));
