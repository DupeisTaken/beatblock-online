import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { resolve } from 'node:path';

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
