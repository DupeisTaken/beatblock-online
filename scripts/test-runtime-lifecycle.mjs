import { spawn, spawnSync } from 'node:child_process';
import { connect } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { buildRuntimeLifecycleReport } from './runtime-lifecycle-report.mjs';

const root = resolve(import.meta.dirname, '..');
const cargoTarget = resolve(root, process.env.CARGO_TARGET_DIR ?? 'companion/target');
const explicitRuntime = process.env.BBT_RUNTIME_EXE;
const executable = explicitRuntime
  ? resolve(explicitRuntime)
  : resolve(cargoTarget, 'release/BeatblockOnlineRuntime.exe');
const data = resolve(root, '.test/runtime-lifecycle-data');
const reportPath = resolve(root, 'reports/trial-runs/runtime-lifecycle-latest.json');
await rm(data, { recursive: true, force: true });
await mkdir(data, { recursive: true });

// The aggregate trial passes the runtime it built in the immediately preceding
// step. Standalone lifecycle runs still build from the current source tree.
if (!explicitRuntime) {
  const build = spawnSync(
    'cargo',
    [
      'build',
      '--manifest-path',
      resolve(root, 'companion/Cargo.toml'),
      '--release',
      '--locked',
      '--bin',
      'BeatblockOnlineRuntime',
    ],
    { cwd: root, encoding: 'utf8', windowsHide: true },
  );
  if (build.status !== 0) {
    throw new Error(`could not build the lifecycle runtime:\n${build.stdout}\n${build.stderr}`);
  }
}

const delay = (ms) => new Promise((resolveDelay) => setTimeout(resolveDelay, ms));
// Windows security/indexing can briefly retain a freshly linked executable
// after Cargo exits. Confirm the child reached its spawn event and retry only
// transient file-lock failures; all other launch errors remain immediate.
async function spawnWithTransientRetry(args, options) {
  let lastError;
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      const child = spawn(executable, args, options);
      await new Promise((resolveSpawn, rejectSpawn) => {
        child.once('spawn', resolveSpawn);
        child.once('error', rejectSpawn);
      });
      return child;
    } catch (error) {
      lastError = error;
      if (!['UNKNOWN', 'EBUSY', 'EACCES'].includes(error?.code) || attempt === 4) throw error;
      await delay(250 * (attempt + 1));
    }
  }
  throw lastError;
}
async function writeFileWithTransientRetry(path, contents) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      await writeFile(path, contents);
      return;
    } catch (error) {
      if (!['UNKNOWN', 'EBUSY', 'EACCES'].includes(error?.code) || attempt === 4) throw error;
      await delay(250 * (attempt + 1));
    }
  }
}
const waitExit = (child, timeout = 5000) =>
  Promise.race([
    new Promise((resolveExit) => child.once('exit', (code) => resolveExit(code))),
    delay(timeout).then(() => {
      child.kill();
      throw new Error('runtime did not exit in time');
    }),
  ]);

async function connectIpc(timeout = 5000) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    try {
      return await new Promise((resolveSocket, reject) => {
        const socket =
          process.platform === 'win32'
            ? connect('\\\\.\\pipe\\beatblock-online-v3', () => resolveSocket(socket))
            : connect(8975, '127.0.0.1', () => resolveSocket(socket));
        socket.once('error', reject);
      });
    } catch {
      await delay(50);
    }
  }
  throw new Error('runtime IPC did not become ready');
}

const runtime = await spawnWithTransientRetry(
  ['--data-dir', data, '--port', '18974', '--session-id', 'lifecycle-trial'],
  {
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  },
);
const socket = await connectIpc();
let incoming = '';
const readyPromise = new Promise((resolveReady, reject) => {
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
socket.write(
  `${JSON.stringify({ version: 3, type: 'client.hello', sequence: 0, runTimeUs: 0, payload: { instanceId: 'lifecycle-trial', clientVersion: 'test', gameVersion: '1.7.1a (Early Access)[d40b7083]', distribution: 'standalone' } })}\n`,
);
const ready = await readyPromise;
if (ready.type !== 'runtime.ready' || ready.version !== 3)
  throw new Error('runtime emitted an incompatible readiness message');

await delay(750);
const processSample = spawnSync(
  'powershell',
  ['-NoProfile', '-Command', `(Get-Process -Id ${runtime.pid}).WorkingSet64`],
  { encoding: 'utf8', windowsHide: true },
);
if (processSample.status !== 0) {
  throw new Error(`could not sample runtime working set: ${processSample.stderr}`);
}
const workingSetBytes = Number(processSample.stdout.trim());

const duplicate = await spawnWithTransientRetry(['--data-dir', data, '--port', '18975'], {
  windowsHide: true,
  stdio: 'ignore',
});
const duplicateCode = await waitExit(duplicate, 3000);
if (duplicateCode === 0)
  throw new Error('duplicate runtime unexpectedly acquired the per-user mutex');

socket.write(
  `${JSON.stringify({ version: 3, type: 'runtime.session_end', sequence: 1, runTimeUs: 1, requestId: 'trial-stop', payload: { requestId: 'trial-stop' } })}\n`,
);
const shutdownCode = await waitExit(runtime, 5000);
socket.destroy();
if (shutdownCode !== 0) throw new Error(`runtime shutdown returned ${shutdownCode}`);

const orphan = await spawnWithTransientRetry(
  ['--data-dir', data, '--port', '18976', '--parent-pid', '4294967294'],
  { windowsHide: true, stdio: 'ignore' },
);
const orphanCode = await waitExit(orphan, 3000);
if (orphanCode !== 0) throw new Error(`parent cleanup returned ${orphanCode}`);

const report = buildRuntimeLifecycleReport({
  readiness: ready.type,
  protocolVersion: ready.version,
  workingSetBytes,
  duplicateMutexRejected: true,
  explicitSessionShutdown: true,
  parentExitCleanup: true,
  visibleConsoleRequested: false,
});
await mkdir(resolve(root, 'reports/trial-runs'), { recursive: true });
await writeFileWithTransientRetry(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
if (!report.passed) {
  throw new Error('runtime lifecycle or resource gate failed; see the report above');
}
