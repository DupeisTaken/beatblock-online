import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { test } from 'node:test';
import {
  assertExactZipEntries,
  checksumLine,
  inspectObsBuildManifest,
  inspectPe,
  inspectZip,
  listZipEntries,
  sha256Hex,
} from './verify-release.mjs';
import { validateReleaseTag } from './verify-release-tag.mjs';

const root = resolve(import.meta.dirname, '..');

test('release version metadata and generated asset names stay aligned', async () => {
  const [
    rootPackageText,
    protocolPackageText,
    cargoManifest,
    cargoLock,
    modCore,
    packageScript,
    modTest,
    uiHarness,
    releaseGuide,
  ] = await Promise.all([
    readFile(resolve(root, 'package.json'), 'utf8'),
    readFile(resolve(root, 'protocol/package.json'), 'utf8'),
    readFile(resolve(root, 'companion/Cargo.toml'), 'utf8'),
    readFile(resolve(root, 'companion/Cargo.lock'), 'utf8'),
    readFile(resolve(root, 'mod/shared/bbt/core.lua'), 'utf8'),
    readFile(resolve(root, 'scripts/package-mods.mjs'), 'utf8'),
    readFile(resolve(root, 'scripts/test-mod.mjs'), 'utf8'),
    readFile(resolve(root, 'tests/ui-harness/main.lua'), 'utf8'),
    readFile(resolve(root, 'docs/releasing.md'), 'utf8'),
  ]);
  const rootPackage = JSON.parse(rootPackageText);
  const protocolPackage = JSON.parse(protocolPackageText);
  const version = rootPackage.version;

  assert.match(version, /^\d+\.\d+\.\d+-alpha\.\d+$/);
  assert.equal(protocolPackage.version, version);
  assert.equal(cargoManifest.match(/^version = "([^"]+)"$/m)?.[1], version);
  assert.equal(
    cargoLock.match(
      /\[\[package\]\]\r?\nname = "beatblock-online-companion"\r?\nversion = "([^"]+)"/,
    )?.[1],
    version,
  );
  assert.equal(modCore.match(/version = '([^']+)'/)?.[1], version);

  // Build, validation, UI fixtures, and operator docs must all identify the
  // same prerelease or a local build can silently publish mixed-version files.
  for (const [label, contents] of [
    ['mod packager', packageScript],
    ['mod packaging gate', modTest],
    ['UI harness', uiHarness],
    ['release guide', releaseGuide],
  ]) {
    assert.ok(contents.includes(version), `${label} does not reference version ${version}`);
  }
  assert.ok(releaseGuide.includes(`v${version}`));
});

test('tagged releases must exactly match the package version', () => {
  assert.deepEqual(
    validateReleaseTag({ refType: 'tag', refName: 'v0.3.0-alpha.3', version: '0.3.0-alpha.3' }),
    { tagged: true, expected: 'v0.3.0-alpha.3' },
  );
  assert.throws(
    () =>
      validateReleaseTag({
        refType: 'tag',
        refName: 'v0.3.0-alpha.4',
        version: '0.3.0-alpha.3',
      }),
    /does not match package version/,
  );
  assert.deepEqual(
    validateReleaseTag({ refType: 'branch', refName: 'main', version: '0.3.0-alpha.3' }),
    { tagged: false, expected: 'v0.3.0-alpha.3' },
  );
});

function x64PeFixture() {
  const buffer = Buffer.alloc(128);
  buffer.write('MZ', 0, 'ascii');
  buffer.writeUInt32LE(64, 0x3c);
  buffer.write('PE\0\0', 64, 'binary');
  buffer.writeUInt16LE(0x8664, 68);
  return buffer;
}

function zipDirectoryFixture(names) {
  const localHeader = Buffer.from([0x50, 0x4b, 0x03, 0x04]);
  const directory = names.map((name) => {
    const encodedName = Buffer.from(name);
    const entry = Buffer.alloc(46 + encodedName.length);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(encodedName.length, 28);
    encodedName.copy(entry, 46);
    return entry;
  });
  const centralSize = directory.reduce((total, entry) => total + entry.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(names.length, 8);
  end.writeUInt16LE(names.length, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(localHeader.length, 16);
  return Buffer.concat([localHeader, ...directory, end]);
}

test('inspectPe accepts an x64 Portable Executable', () => {
  assert.deepEqual(inspectPe(x64PeFixture(), 'fixture.exe'), {
    machine: 0x8664,
    size: 128,
  });
});

test('inspectPe rejects malformed and non-x64 binaries', () => {
  assert.throws(() => inspectPe(Buffer.from('not a PE')), /missing MZ header/);
  const x86 = x64PeFixture();
  x86.writeUInt16LE(0x14c, 68);
  assert.throws(() => inspectPe(x86), /expected x64/);
});

test('inspectZip accepts ZIP signatures and rejects other files', () => {
  assert.deepEqual(inspectZip(Buffer.from([0x50, 0x4b, 0x03, 0x04])), { size: 4 });
  assert.throws(() => inspectZip(Buffer.from('not a zip')), /not a ZIP archive/);
});

test('listZipEntries reads a ZIP central directory without platform tools', () => {
  const archive = zipDirectoryFixture(['BeatblockOnline/', 'BeatblockOnline/bbt/core.lua']);

  assert.deepEqual(listZipEntries(archive), ['BeatblockOnline/', 'BeatblockOnline/bbt/core.lua']);
});

test('listZipEntries rejects malformed central-directory metadata', () => {
  const archive = zipDirectoryFixture(['BeatblockOnline/main.lua']);
  archive.writeUInt32LE(123, archive.length - 6);

  assert.throws(() => listZipEntries(archive), /malformed ZIP central directory/);
});

test('final ZIP verification rejects missing or unexpected files', () => {
  const expected = ['BeatblockOnline/README.txt', 'BeatblockOnline/bbt/core.lua'];
  assert.deepEqual(
    assertExactZipEntries(
      ['BeatblockOnline\\bbt\\core.lua', 'BeatblockOnline\\README.txt'],
      expected,
    ),
    expected,
  );
  assert.throws(
    () =>
      assertExactZipEntries(
        ['BeatblockOnline/README.txt', 'BeatblockOnline/unknown.dll'],
        expected,
        'fixture.zip',
      ),
    /contents differ from the reviewed source tree/,
  );
});

test('hosted workflows preserve source-only and artifact-backed test boundaries', async () => {
  const [ci, release, auditPolicy] = await Promise.all([
    readFile(resolve(root, '.github/workflows/ci.yml'), 'utf8'),
    readFile(resolve(root, '.github/workflows/release.yml'), 'utf8'),
    readFile(resolve(root, 'companion/.cargo/audit.toml'), 'utf8'),
  ]);
  for (const testName of [
    'detector_accepts_isolated_test_game_shape',
    'embedded_obs_source_is_a_real_module_with_required_exports',
    'full_standalone_install_repair_restore_and_uninstall_round_trip',
    'move_installation_and_adapter_detection_are_exclusive',
    'verified_obs_component_does_not_report_a_stale_failure',
  ]) {
    assert.match(ci, new RegExp(`--skip ${testName}`));
  }

  const buildStep = release.indexOf('- run: pnpm build\n');
  const fullRustSuite = release.indexOf(
    '- run: cargo test --manifest-path companion/Cargo.toml --lib --bins',
  );
  const pinnedNightly = release.indexOf('toolchain: nightly-2026-07-15');
  const stableReset = release.indexOf('- run: rustup default stable');
  assert.ok(pinnedNightly >= 0, 'release workflow must install the pinned Lovely toolchain');
  assert.ok(
    pinnedNightly < stableReset && stableReset < buildStep,
    'release workflow must restore stable before building and testing the companion',
  );
  assert.ok(buildStep >= 0, 'release workflow must build publishable artifacts');
  assert.ok(
    buildStep < fullRustSuite,
    'release workflow must build payloads before the full Rust suite',
  );
  assert.equal(
    release.indexOf('- run: pnpm build\n', buildStep + 1),
    -1,
    'release workflow should build artifacts exactly once',
  );

  for (const workflow of [ci, release]) {
    assert.match(
      workflow,
      /powershell -NoProfile -File scripts\/test-release-utils\.ps1/,
      'hosted Windows workflows must exercise the legacy-compatible checksum helper',
    );
    assert.match(workflow, /cargo install cargo-audit --version 0\.22\.2 --locked/);
    assert.match(workflow, /- run: cargo audit\r?\n\s+working-directory: companion/);
    assert.match(
      workflow,
      /cargo clippy --manifest-path companion\/Cargo\.toml --all-targets -- -D warnings/,
    );
    assert.match(
      workflow,
      /cargo clippy --manifest-path companion\/Cargo\.toml --features installer-ui --bin BeatblockOnlineInstaller -- -D warnings/,
    );
  }
  assert.match(auditPolicy, /os = \["windows"\]/);
  assert.match(auditPolicy, /RUSTSEC-2026-0194/);
  assert.match(auditPolicy, /RUSTSEC-2026-0195/);
});

test('hosted workflows pin third-party actions and isolate release publication', async () => {
  const workflows = await Promise.all(
    ['ci.yml', 'release.yml'].map((name) =>
      readFile(resolve(root, '.github/workflows', name), 'utf8'),
    ),
  );
  for (const workflow of workflows) {
    const uses = [...workflow.matchAll(/^\s*-\s+uses:\s+([^\s#]+)/gm)].map((match) => match[1]);
    assert.ok(uses.length > 0);
    for (const action of uses) {
      assert.match(action, /@[0-9a-f]{40}$/, `${action} is not pinned to a full commit SHA`);
    }
  }

  const release = workflows[1];
  const buildJob = release.slice(release.indexOf('  build:'), release.indexOf('  publish:'));
  const publishJob = release.slice(release.indexOf('  publish:'));
  assert.doesNotMatch(buildJob, /contents:\s*write/);
  assert.match(publishJob, /needs:\s*build/);
  assert.match(publishJob, /contents:\s*write/);
  assert.match(release, /actions\/attest-build-provenance@[0-9a-f]{40}/);
});

test('checksumLine is stable and uses only the asset filename', () => {
  assert.equal(
    checksumLine('nested/asset.bin', Buffer.from('release')),
    'a4d451ec23463726f72c43d64c710968f6b602cd653b4de8adee1b556240a829  asset.bin',
  );
});

test('OBS build manifest binds the native artifact to the reviewed source', () => {
  const source = Buffer.from('gs_effect_set_texture_srgb(image, ctx->texture);');
  const artifact = x64PeFixture();
  const manifest = {
    schemaVersion: 1,
    obsVersion: '32.0.4',
    sourcePath: 'obs-plugin/src/plugin.c',
    sourceSha256: sha256Hex(source),
    artifactSha256: sha256Hex(artifact),
  };

  assert.deepEqual(inspectObsBuildManifest(manifest, artifact, source), {
    sourceSha256: manifest.sourceSha256,
    artifactSha256: manifest.artifactSha256,
    obsVersion: '32.0.4',
  });
  assert.throws(
    () => inspectObsBuildManifest(manifest, artifact, Buffer.from('changed source')),
    /stale for the current plugin source/,
  );
  assert.throws(
    () => inspectObsBuildManifest(manifest, Buffer.from('changed artifact'), source),
    /artifact hash does not match/,
  );
  assert.throws(
    () => inspectObsBuildManifest({ ...manifest, schemaVersion: 2 }, artifact, source),
    /manifest is malformed/,
  );
  assert.throws(
    () => inspectObsBuildManifest({ ...manifest, obsVersion: '31.1.2' }, artifact, source),
    /manifest is malformed/,
  );
});

test('OBS build and installer stages enforce native source provenance', async () => {
  const [buildObs, buildWindows] = await Promise.all([
    readFile(resolve(root, 'scripts/build-obs-plugin.ps1'), 'utf8'),
    readFile(resolve(root, 'scripts/build-windows.mjs'), 'utf8'),
  ]);
  assert.match(buildObs, /ChangeExtension\(\$OutputPath, '\.build\.json'\)/);
  assert.match(buildObs, /sourceSha256 = \$sourceHash\.ToLowerInvariant\(\)/);
  assert.match(buildObs, /artifactSha256 = \$artifactHash\.ToLowerInvariant\(\)/);

  const provenanceCheck = buildWindows.indexOf('await verifyObsBuildManifest({');
  const runtimeBuild = buildWindows.indexOf(
    "cargo(['build', '--manifest-path', manifest, '--release', '--bin', 'BeatblockOnlineRuntime'])",
  );
  assert.ok(provenanceCheck >= 0, 'installer build must verify OBS provenance');
  assert.ok(
    provenanceCheck < runtimeBuild,
    'stale OBS artifacts must fail before expensive Rust release compilation',
  );
});

test('installer builds keep one canonical release directory', async () => {
  const [buildWindows, gitignore, prettierignore] = await Promise.all([
    readFile(resolve(root, 'scripts/build-windows.mjs'), 'utf8'),
    readFile(resolve(root, '.gitignore'), 'utf8'),
    readFile(resolve(root, '.prettierignore'), 'utf8'),
  ]);

  assert.doesNotMatch(buildWindows, /localReleases|resolve\(root, 'releases'\)/);
  assert.match(
    buildWindows,
    /copyFile\(installer, resolve\(release, 'BeatblockOnlineInstaller\.exe'\)\)/,
  );
  assert.doesNotMatch(buildWindows, /BeatblockOnlineInstaller-[^']+\.exe/);
  for (const ignoreFile of [gitignore, prettierignore]) {
    assert.match(ignoreFile, /^\/release\/$/m);
    assert.doesNotMatch(ignoreFile, /^\/?releases\/$/m);
  }
});
