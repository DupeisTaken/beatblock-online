import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { cargoCommand } from './run-cargo.mjs';

const root = resolve(import.meta.dirname, '..');
const reportPath = resolve(root, 'reports', 'trial-runs', 'runtime-benchmark-latest.json');

// The benchmark has `harness = false`, so Cargo's release test profile runs
// the real workload while avoiding a redundant transient bench-profile binary.
const benchmark = spawnSync(
  cargoCommand(),
  ['test', '--manifest-path', 'companion/Cargo.toml', '--release', '--bench', 'companion_bench'],
  {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, BBT_BENCH_REPORT: reportPath, CI: 'true' },
  },
);

if (benchmark.error) throw benchmark.error;
if (benchmark.status !== 0) process.exit(benchmark.status ?? 1);

const verification = spawnSync(process.execPath, ['scripts/verify-benchmark-report.mjs'], {
  cwd: root,
  stdio: 'inherit',
});

if (verification.error) throw verification.error;
process.exit(verification.status ?? 1);
