import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { test } from 'node:test';
import { checksumLine, inspectPe, inspectZip, listZipEntries } from './verify-release.mjs';

const root = resolve(import.meta.dirname, '..');

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

test('hosted workflows preserve source-only and artifact-backed test boundaries', async () => {
  const [ci, release] = await Promise.all([
    readFile(resolve(root, '.github/workflows/ci.yml'), 'utf8'),
    readFile(resolve(root, '.github/workflows/release.yml'), 'utf8'),
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
  }
});

test('checksumLine is stable and uses only the asset filename', () => {
  assert.equal(
    checksumLine('nested/asset.bin', Buffer.from('release')),
    'a4d451ec23463726f72c43d64c710968f6b602cd653b4de8adee1b556240a829  asset.bin',
  );
});
