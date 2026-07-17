import assert from 'node:assert/strict';
import { test } from 'node:test';
import { inspectPe, inspectZip, checksumLine } from './verify-release.mjs';

function x64PeFixture() {
  const buffer = Buffer.alloc(128);
  buffer.write('MZ', 0, 'ascii');
  buffer.writeUInt32LE(64, 0x3c);
  buffer.write('PE\0\0', 64, 'binary');
  buffer.writeUInt16LE(0x8664, 68);
  return buffer;
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

test('checksumLine is stable and uses only the asset filename', () => {
  assert.equal(
    checksumLine('nested/asset.bin', Buffer.from('release')),
    'a4d451ec23463726f72c43d64c710968f6b602cd653b4de8adee1b556240a829  asset.bin',
  );
});
