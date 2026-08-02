import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function releaseDisplayTitle(version, compatibility) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`Cannot build a release title from invalid SemVer tag ${version}`);
  }
  const testedVersion = compatibility?.testedVersion;
  if (typeof testedVersion !== 'string' || !/^\d+\.\d+\.\d+[A-Za-z]?$/.test(testedVersion)) {
    throw new Error('package.json beatblockCompatibility.testedVersion is invalid');
  }
  if (compatibility.newerBuilds !== 'accepted-unverified') {
    throw new Error('Beatblock compatibility policy must explicitly describe newer builds');
  }
  // Releases and in-game builds keep the familiar player-facing `v` label.
  return `v${version} for Beatblock ${testedVersion}+`;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const root = resolve(import.meta.dirname, '..');
  const manifest = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
  const version = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? manifest.version;
  process.stdout.write(`${releaseDisplayTitle(version, manifest.beatblockCompatibility)}\n`);
}
