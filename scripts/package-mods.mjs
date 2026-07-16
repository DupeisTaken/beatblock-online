import { cp, mkdir, rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve(import.meta.dirname, '..');
const shared = resolve(root, 'mod/shared');
const distributions = ['standalone', 'beatblock-plus'];

for (const name of distributions) {
  const target = resolve(root, `mod/${name}`);
  await rm(resolve(target, 'bbt'), { recursive: true, force: true });
  await mkdir(resolve(target, 'lovely'), { recursive: true });
  await cp(resolve(shared, 'bbt'), resolve(target, 'bbt'), { recursive: true });
  await cp(resolve(shared, 'lovely/hooks.toml'), resolve(target, 'lovely/hooks.toml'));
}

const releases = resolve(root, 'mod/releases');
await rm(releases, { recursive: true, force: true });
await mkdir(releases, { recursive: true });
for (const name of distributions) {
  const target = resolve(root, `mod/${name}`);
  const stage = resolve(releases, `.stage-${name}`);
  const stagedMod = resolve(stage, 'BeatblockTogether');
  await mkdir(stage, { recursive: true });
  await cp(target, stagedMod, { recursive: true });
  const archive = resolve(releases, `beatblock-together-${name}-0.3.0-alpha.1.zip`);
  const result =
    process.platform === 'win32'
      ? spawnSync(
          'powershell',
          [
            '-NoProfile',
            '-Command',
            `Compress-Archive -Path '${stagedMod.replaceAll("'", "''")}' -DestinationPath '${archive.replaceAll("'", "''")}' -Force`,
          ],
          { stdio: 'inherit' },
        )
      : spawnSync('zip', ['-qr', archive, 'BeatblockTogether'], {
          cwd: stage,
          stdio: 'inherit',
        });
  if (result.status !== 0) throw new Error(`Failed to package ${name}`);
  await rm(stage, { recursive: true, force: true });
}

console.log('Generated standalone and BeatblockPlus mod trees and release ZIPs.');
