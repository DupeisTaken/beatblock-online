import { access, copyFile, mkdir, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { cargoCommand } from './run-cargo.mjs';
import { obsBuildManifestPath, verifyObsBuildManifest } from './verify-release.mjs';

const root = resolve(import.meta.dirname, '..');
const manifest = resolve(root, 'companion/Cargo.toml');
// Respect Cargo's target override for release staging. This lets maintainers
// rebuild an installer while a repo-local test runtime is still executing,
// without terminating that game or embedding an older locked executable.
const cargoTarget = resolve(root, process.env.CARGO_TARGET_DIR ?? 'companion/target');
const runtime = resolve(cargoTarget, 'release/BeatblockOnlineRuntime.exe');
const installer = resolve(cargoTarget, 'release/BeatblockOnlineInstaller.exe');
const lovely = resolve(process.env.BBT_LOVELY_DLL ?? resolve(root, 'artifacts/lovely/version.dll'));
const obsPlugin = resolve(
  process.env.BBT_OBS_PLUGIN_DLL ?? resolve(root, 'artifacts/obs/beatblock-online-obs.dll'),
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

// Build order is intentional: the one downloadable installer embeds the
// generated, verified dependencies and the lean runtime byte-for-byte.
for (const [name, path] of [
  ['Lovely injector', lovely],
  ['OBS source', obsPlugin],
]) {
  await access(path).catch(() => {
    throw new Error(`${name} artifact is missing: ${path}. Run pnpm build.`);
  });
}
await verifyObsBuildManifest({
  pluginPath: obsPlugin,
  sourcePath: resolve(root, 'obs-plugin/src/plugin.c'),
  manifestPath: obsBuildManifestPath(obsPlugin),
}).catch((error) => {
  throw new Error(
    `Refusing to embed an unverified or stale OBS source. Run pnpm build:obs and retry. ${error.message}`,
    { cause: error },
  );
});
cargo(['build', '--manifest-path', manifest, '--release', '--bin', 'BeatblockOnlineRuntime']);
cargo(
  [
    'build',
    '--manifest-path',
    manifest,
    '--release',
    '--bin',
    'BeatblockOnlineInstaller',
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

// Self-update relies on this exact bounded probe before a downloaded binary is
// allowed to replace the managed installer. Exercise the release artifact now
// so a tagged build cannot publish an executable that breaks that contract.
const cargoManifest = await readFile(manifest, 'utf8');
const expectedVersion = cargoManifest.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m)?.[1];
if (!expectedVersion) throw new Error('Could not read the installer version from Cargo.toml');
const versionProbe = spawnSync(installer, ['--version'], {
  cwd: root,
  encoding: 'utf8',
  timeout: 7000,
  windowsHide: true,
});
if (versionProbe.error) throw versionProbe.error;
if (versionProbe.status !== 0 || !versionProbe.stdout.split(/\s+/).includes(expectedVersion)) {
  throw new Error(
    `Installer --version probe did not report ${expectedVersion}: ${versionProbe.stdout || versionProbe.stderr}`,
  );
}

// Keep the staging directory itself stable: Explorer, antivirus scanners, and
// terminals can hold a Windows directory handle even when the output file is
// replaceable. Each release artifact is explicitly overwritten below.
await mkdir(release, { recursive: true });
// The GitHub staging artifact is also the only local review copy. Keeping one
// canonical path prevents stale installers from surviving in a sibling folder.
await copyFile(installer, resolve(release, 'BeatblockOnlineInstaller.exe'));
console.log(
  'Built release/BeatblockOnlineInstaller.exe (self-contained installer + runtime payload).',
);
