import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import test from 'node:test';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const validator = resolve(import.meta.dirname, 'validate-patches.mjs');

test('source-only patch validation does not require proprietary game archives', () => {
  const output = execFileSync(process.execPath, [validator, '--source-only'], {
    encoding: 'utf8',
  });

  assert.match(output, /Lovely patch manifest signatures \(source-only\)/);
});

test('patch validation rejects unknown modes instead of weakening the gate', () => {
  const result = spawnSync(process.execPath, [validator, '--allow-missing-fixture'], {
    encoding: 'utf8',
  });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Unsupported patch-validation argument/);
});

test('full patch validation accepts an explicit isolated game fixture', () => {
  assert.match(readFileSync(validator, 'utf8'), /process\.env\.BBT_GAME_FIXTURE/);
});

test('full patch validation rejects a fixture that does not match the pinned hashes', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'bbo-patch-fixture-'));
  try {
    mkdirSync(resolve(fixture, 'packed'));
    writeFileSync(resolve(fixture, 'Beatblock.exe'), 'not the pinned executable');
    writeFileSync(resolve(fixture, 'packed/obj.zip'), 'not the pinned object archive');
    writeFileSync(resolve(fixture, 'packed/states.zip'), 'not the pinned state archive');

    const result = spawnSync(process.execPath, [validator], {
      encoding: 'utf8',
      env: { ...process.env, BBT_GAME_FIXTURE: fixture },
    });

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Pinned fixture hash mismatch for Beatblock\.exe/);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});
