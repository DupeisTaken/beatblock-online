import { spawn, spawnSync } from 'node:child_process';
import { connect } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const executable = resolve(root, 'companion/target/release/BeatblockTogetherRuntime.exe');
const data = resolve(root, '.test/runtime-lifecycle-data');
const reportPath = resolve(root, 'reports/trial-runs/runtime-lifecycle-latest.json');
await rm(data, { recursive: true, force: true });
await mkdir(data, { recursive: true });

const delay = (ms) => new Promise((resolveDelay) => setTimeout(resolveDelay, ms));
const waitExit = (child, timeout = 5000) => Promise.race([
  new Promise((resolveExit) => child.once('exit', (code) => resolveExit(code))),
  delay(timeout).then(() => { child.kill(); throw new Error('runtime did not exit in time'); }),
]);

async function connectIpc(timeout = 5000) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    try {
      return await new Promise((resolveSocket, reject) => {
        const socket = connect(8975, '127.0.0.1', () => resolveSocket(socket));
        socket.once('error', reject);
      });
    } catch { await delay(50); }
  }
  throw new Error('runtime IPC did not become ready');
}

const runtime = spawn(executable, ['--data-dir', data, '--port', '18974', '--session-id', 'lifecycle-trial'], {
  windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
});
const socket = await connectIpc();
let incoming = '';
const ready = await new Promise((resolveReady, reject) => {
  const timer = setTimeout(() => reject(new Error('runtime.ready was not received')), 3000);
  socket.on('data', (chunk) => {
    incoming += chunk.toString('utf8');
    const newline = incoming.indexOf('\n');
    if (newline >= 0) {
      clearTimeout(timer);
      resolveReady(JSON.parse(incoming.slice(0, newline)));
      incoming = incoming.slice(newline + 1);
    }
  });
});
if (ready.type !== 'runtime.ready' || ready.version !== 2) throw new Error('runtime emitted an incompatible readiness message');

await delay(750);
const processSample = spawnSync('powershell', ['-NoProfile', '-Command', `(Get-Process -Id ${runtime.pid}).WorkingSet64`], { encoding: 'utf8', windowsHide: true });
const workingSetBytes = Number(processSample.stdout.trim());

const duplicate = spawn(executable, ['--data-dir', data, '--port', '18975'], { windowsHide: true, stdio: 'ignore' });
const duplicateCode = await waitExit(duplicate, 3000);
if (duplicateCode === 0) throw new Error('duplicate runtime unexpectedly acquired the per-user mutex');

socket.write(`${JSON.stringify({ version: 2, type: 'runtime.session_end', sequence: 1, runTimeUs: 1, requestId: 'trial-stop', payload: { requestId: 'trial-stop' } })}\n`);
const shutdownCode = await waitExit(runtime, 5000);
socket.destroy();
if (shutdownCode !== 0) throw new Error(`runtime shutdown returned ${shutdownCode}`);

const orphan = spawn(executable, ['--data-dir', data, '--port', '18976', '--parent-pid', '4294967294'], { windowsHide: true, stdio: 'ignore' });
const orphanCode = await waitExit(orphan, 3000);
if (orphanCode !== 0) throw new Error(`parent cleanup returned ${orphanCode}`);

const report = {
  schemaVersion: 2,
  generatedAt: new Date().toISOString(),
  passed: true,
  readiness: ready.type,
  protocolVersion: ready.version,
  workingSetBytes,
  idleWorkingSetTargetBytes: 30 * 1024 * 1024,
  idleWorkingSetTargetMet: workingSetBytes > 0 && workingSetBytes < 30 * 1024 * 1024,
  duplicateMutexRejected: true,
  explicitSessionShutdown: true,
  parentExitCleanup: true,
  visibleConsoleRequested: false,
};
await mkdir(resolve(root, 'reports/trial-runs'), { recursive: true });
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
