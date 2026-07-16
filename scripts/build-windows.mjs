import { access, copyFile, mkdir, rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { cargoCommand } from './run-cargo.mjs';

const root = resolve(import.meta.dirname, '..');
const manifest = resolve(root, 'companion/Cargo.toml');
const runtime = resolve(root, 'companion/target/release/BeatblockOnlineRuntime.exe');
const installer = resolve(root, 'companion/target/release/BeatblockTogetherInstaller.exe');
const lovely = resolve(root, '.reference/lovely-injector/target/release/version.dll');
const obsPlugin = resolve(
  root,
  'obs-plugin/artifacts/obs-32.0.4/beatblock-together-obs.dll',
);
const release = resolve(root, 'release');

function cargo(args, env = {}) {
  const result = spawnSync(cargoCommand(), args, {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, ...env },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Cargo failed with exit code ${result.status}`);
}

// Build order is intentional: the one downloadable installer embeds the lean
// runtime artifact byte-for-byte and installs it only as an Online dependency.
await access(obsPlugin).catch(() => {
  throw new Error(`Reviewed OBS source artifact is missing: ${obsPlugin}`);
});
cargo(['build', '--manifest-path', manifest, '--release', '--bin', 'BeatblockOnlineRuntime']);
cargo(
  [
    'build',
    '--manifest-path',
    manifest,
    '--release',
    '--bin',
    'BeatblockTogetherInstaller',
    '--features',
    'installer-ui',
  ],
  {
    BBT_RUNTIME_EXE: runtime,
    // Release builds use the reviewed BBT fork: silent for normal Steam
    // launches, with --enable-console retained as an explicit developer aid.
    BBT_LOVELY_DLL: lovely,
    BBT_OBS_PLUGIN_DLL: obsPlugin,
  },
);

await rm(release, { recursive: true, force: true });
await mkdir(release, { recursive: true });
await copyFile(installer, resolve(release, 'BeatblockTogetherInstaller.exe'));
console.log(
  'Built release/BeatblockTogetherInstaller.exe (self-contained installer + runtime payload).',
);
