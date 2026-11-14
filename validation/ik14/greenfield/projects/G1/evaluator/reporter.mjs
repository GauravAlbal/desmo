const IDS_R = ['R1','R2','R3','R4','R5','R6','R7','R8','R9','R10','R11','R12'];
const IDS_C = ['C1','C2','C3','C4'];

export default async function * reporter(source) {
  const scope = process.env.EVAL_SCOPE === 'brief' ? 'brief' : 'all';
  const ids = scope === 'brief' ? IDS_R : IDS_R.concat(IDS_C);
  const status = new Map(ids.map((id) => [id, 'missing']));
  for await (const event of source) {
    if (event.type !== 'test:pass' && event.type !== 'test:fail') continue;
    const name = event.data && event.data.name ? event.data.name : '';
    const match = /^([RC]\d+)\b/.exec(name);
    if (!match) continue;
    if (event.data.skip) continue;
    const id = match[1];
    if (!status.has(id)) continue;
    if (event.type === 'test:fail') {
      status.set(id, 'fail');
      const err = event.data.details && event.data.details.error;
      const message = err && err.message ? err.message : 'failed';
      console.error(id + ' FAIL ' + message.split('\n')[0]);
    } else if (status.get(id) !== 'fail') {
      status.set(id, 'pass');
    }
  }
  const requirements = {};
  let allPassed = true;
  for (const id of ids) {
    requirements[id] = status.get(id);
    if (requirements[id] !== 'pass') allPassed = false;
  }
  yield JSON.stringify({ scope, all_passed: allPassed, requirements }, null, 2) + '\n';
}
