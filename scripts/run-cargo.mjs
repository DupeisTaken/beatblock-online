import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join } from 'node:path';

export function cargoCommand() {
  const installed =
    process.platform === 'win32'
      ? join(process.env.USERPROFILE ?? homedir(), '.cargo', 'bin', 'cargo.exe')
      : join(homedir(), '.cargo', 'bin', 'cargo');
  return existsSync(installed) ? installed : 'cargo';
}

if (process.argv[1]?.endsWith('run-cargo.mjs')) {
  const result = spawnSync(cargoCommand(), process.argv.slice(2), { stdio: 'inherit' });
  if (result.error) throw result.error;
  process.exit(result.status ?? 1);
}
