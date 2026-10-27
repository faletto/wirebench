// CI check of the engine worker: run the load test with the engine in the worker and in the page and
// fail if the worker was not used or made the page's frames noticeably worse. Compared against the
// inline run on the same machine, so a slow runner does not matter. The regression this guards
// against (a clock loop at full speed) made the median gap six times worse.
//   CHROMIUM=... node check.mjs [rate=8] [url=http://localhost:8765/] [lab=./hundred-lab.toml]
import { execFileSync } from 'child_process';
const rate = process.argv[2] ?? '8';
const base = process.argv[3] ?? 'http://localhost:8765/';
const lab = process.argv[4] ?? './hundred-lab.toml';
const run = (url, out) => {
  const text = execFileSync('node', ['load.mjs', rate, url, out, lab], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
  console.log(text.replace(/^RESULT .*\n/m, ''));
  return JSON.parse(text.split('\n').find(l => l.startsWith('RESULT ')).slice(7));
};
const inline = run(base + '?engine=inline', 'out-load-inline');
const worker = run(base, 'out-load');
const fail = [];
if (!worker.inWorker) fail.push('the engine did not start in a worker');
if (inline.inWorker) fail.push('?engine=inline still started a worker');
for (const phase of ['live', 'after']) {
  const w = worker[phase], i = inline[phase];
  console.log(`${phase}: worker p50 ${w.p50} p99 ${w.p99}; inline p50 ${i.p50} p99 ${i.p99}`);
  if (w.p50 > 2 * i.p50 + 4) fail.push(`${phase}: worker median frame gap ${w.p50} ms vs ${i.p50} ms inline`);
  if (w.p99 > 2 * i.p99 + 10) fail.push(`${phase}: worker 99th percentile frame gap ${w.p99} ms vs ${i.p99} ms inline`);
}
if (fail.length) { console.error('FAIL\n' + fail.join('\n')); process.exit(1); }
console.log('ok');
