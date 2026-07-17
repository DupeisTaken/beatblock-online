import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { cargoCommand } from './run-cargo.mjs';

const root = resolve(import.meta.dirname, '..');
const reportDirectory = resolve(root, 'reports', 'trial-runs');
await mkdir(reportDirectory, { recursive: true });
const delay = (ms) => new Promise((resolveDelay) => setTimeout(resolveDelay, ms));
async function writeFileWithTransientRetry(path, contents) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      await writeFile(path, contents);
      return;
    } catch (error) {
      // A just-finished Windows executable can trigger short-lived scanner
      // locks on adjacent generated reports. Bound retries to these lock codes.
      if (!['UNKNOWN', 'EBUSY', 'EACCES'].includes(error?.code) || attempt === 4) throw error;
      await delay(250 * (attempt + 1));
    }
  }
}

const commands = [
  {
    name: 'Protocol v2 typecheck',
    command: process.execPath,
    args: ['node_modules/typescript/bin/tsc', '-p', 'protocol/tsconfig.json'],
  },
  {
    name: 'Protocol v2 schema generation',
    command: process.execPath,
    args: ['scripts/generate-protocol.mjs'],
  },
  {
    name: 'Protocol v2 tests',
    command: process.execPath,
    args: ['node_modules/vitest/vitest.mjs', 'run', 'protocol/test/score.test.ts'],
  },
  {
    name: 'Rust runtime, installer, Lua, and stress tests',
    command: cargoCommand(),
    args: [
      'test',
      '--manifest-path',
      'companion/Cargo.toml',
      '--release',
      '--all-targets',
      '--features',
      'installer-ui',
    ],
  },
  {
    name: 'Package both in-game adapters',
    command: process.execPath,
    args: ['scripts/package-mods.mjs'],
  },
  {
    name: 'In-game mod conformance',
    command: process.execPath,
    args: ['scripts/test-mod.mjs'],
  },
  {
    name: 'Hidden runtime lifecycle and resource gate',
    command: process.execPath,
    args: ['scripts/test-runtime-lifecycle.mjs'],
  },
  {
    name: '16-player / 32-spectator direct-host simulation',
    command: cargoCommand(),
    args: [
      'run',
      '--manifest-path',
      'companion/Cargo.toml',
      '--release',
      '--example',
      'host_room_trial',
    ],
  },
  {
    // `cargo test --all-targets` above executes the benchmark target and writes
    // its report. Verify that artifact here instead of compiling the same
    // executable a second time, which Windows Application Control may reject
    // solely because it has a new transient filename.
    name: 'Runtime I/O benchmark report',
    command: process.execPath,
    args: ['scripts/verify-benchmark-report.mjs'],
  },
];

const runs = [];
for (const trial of commands) {
  console.log(`\n=== ${trial.name} ===`);
  const started = Date.now();
  const result = spawnSync(trial.command, trial.args, {
    cwd: root,
    stdio: 'inherit',
    shell: process.platform === 'win32' && trial.command === 'pnpm',
    env: {
      ...process.env,
      BBT_BENCH_REPORT: resolve(reportDirectory, 'runtime-benchmark-latest.json'),
      BBT_HOST_TRIAL_REPORT: resolve(reportDirectory, 'host-room-simulation-latest.json'),
      CI: 'true',
    },
  });
  runs.push({ name: trial.name, passed: result.status === 0, durationMs: Date.now() - started });
  if (result.error) console.error(result.error);
  if (result.status !== 0) break;
}

const readJson = async (name) => {
  try {
    return JSON.parse(await readFile(resolve(reportDirectory, name), 'utf8'));
  } catch {
    return undefined;
  }
};
const report = {
  schemaVersion: 2,
  generatedAt: new Date().toISOString(),
  passed: runs.length === commands.length && runs.every((run) => run.passed),
  environment: { platform: process.platform, architecture: process.arch, node: process.version },
  runs,
  hostRoom: await readJson('host-room-simulation-latest.json'),
  runtime: await readJson('runtime-benchmark-latest.json'),
  lifecycle: await readJson('runtime-lifecycle-latest.json'),
};
await writeFileWithTransientRetry(
  resolve(reportDirectory, 'full-capability-latest.json'),
  `${JSON.stringify(report, null, 2)}\n`,
);
const markdown = `# Beatblock Online installer/runtime capability trial\n\nGenerated: ${report.generatedAt}\n\nAutomated gate: **${report.passed ? 'PASS' : 'FAIL'}**\n\n${runs.map((run) => `- ${run.passed ? 'PASS' : 'FAIL'} - ${run.name}: ${(run.durationMs / 1000).toFixed(2)} s`).join('\n')}\n\nMachine-readable metrics are in \`full-capability-latest.json\`, \`host-room-simulation-latest.json\`, \`runtime-lifecycle-latest.json\`, and \`runtime-benchmark-latest.json\`. Physical WAN, OBS, GPU, and clean-machine release gates use the manual trial sheets under \`docs/trials\`; simulations are not reported as physical results.\n`;
await writeFileWithTransientRetry(resolve(reportDirectory, 'full-capability-latest.md'), markdown);
console.log(`\nFull trial: ${report.passed ? 'PASS' : 'FAIL'}`);
console.log(`Report: ${resolve(reportDirectory, 'full-capability-latest.md')}`);
if (!report.passed) process.exit(1);
