import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { cargoCommand } from './run-cargo.mjs';

const root = resolve(import.meta.dirname, '..');
const reportDirectory = resolve(root, 'reports', 'trial-runs');
const pnpmCli = process.env.npm_execpath;
const pnpmCommand = pnpmCli ? process.execPath : 'pnpm';
const pnpmArgs = (args) => (pnpmCli ? [pnpmCli, ...args] : args);
await mkdir(reportDirectory, { recursive: true });

const commands = [
  {
    name: 'TypeScript unit and stress tests',
    command: pnpmCommand,
    args: pnpmArgs(['test']),
  },
  { name: 'Protocol/build verification', command: pnpmCommand, args: pnpmArgs(['build']) },
  {
    name: 'In-game mod conformance and packaging',
    command: pnpmCommand,
    args: pnpmArgs(['test:mod']),
  },
  {
    name: 'Rust companion and Beatblock Lua runtime tests',
    command: cargoCommand(),
    args: ['test', '--manifest-path', 'companion/Cargo.toml', '--release'],
  },
  {
    name: 'Server maximum-capacity benchmark',
    command: pnpmCommand,
    args: pnpmArgs(['--filter', '@bbt/server', 'benchmark']),
  },
  {
    name: 'Companion I/O benchmark',
    command: cargoCommand(),
    args: ['bench', '--manifest-path', 'companion/Cargo.toml', '--bench', 'companion_bench'],
  },
];

const runs = [];
for (const trial of commands) {
  console.log(`\n=== ${trial.name} ===`);
  const started = Date.now();
  const result = spawnSync(trial.command, trial.args, {
    cwd: root,
    stdio: 'inherit',
    env: {
      ...process.env,
      BBT_BENCH_REPORT: resolve(reportDirectory, 'companion-benchmark-latest.json'),
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
  schemaVersion: 1,
  generatedAt: new Date().toISOString(),
  passed: runs.length === commands.length && runs.every((run) => run.passed),
  environment: { platform: process.platform, architecture: process.arch, node: process.version },
  runs,
  server: await readJson('server-stress-latest.json'),
  companion: await readJson('companion-benchmark-latest.json'),
};
await writeFile(
  resolve(reportDirectory, 'full-capability-latest.json'),
  `${JSON.stringify(report, null, 2)}\n`,
);
const markdown = `# Beatblock Together full-capability trial\n\nGenerated: ${report.generatedAt}\n\nOverall: **${report.passed ? 'PASS' : 'FAIL'}**\n\n${runs.map((run) => `- ${run.passed ? 'PASS' : 'FAIL'} - ${run.name}: ${(run.durationMs / 1000).toFixed(2)} s`).join('\n')}\n\nMachine-readable metrics are in \`full-capability-latest.json\`, \`server-stress-latest.json\`, and \`companion-benchmark-latest.json\`.\n`;
await writeFile(resolve(reportDirectory, 'full-capability-latest.md'), markdown);
console.log(`\nFull trial: ${report.passed ? 'PASS' : 'FAIL'}`);
console.log(`Report: ${resolve(reportDirectory, 'full-capability-latest.md')}`);
if (!report.passed) process.exit(1);
