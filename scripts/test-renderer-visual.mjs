import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const mode = (process.argv[2] ?? 'full').toLowerCase();
assert.ok(mode === 'full' || mode === 'clean', 'renderer mode must be full or clean');
const fixture = resolve(process.env.BBT_UI_FIXTURE ?? resolve(root, '.test/ui-harness'));
const stage = await mkdtemp(resolve(tmpdir(), 'bbt-renderer-'));
const framePath = resolve(stage, 'stream-A.bbtframe');
const statePath = resolve(stage, 'stream-A.bbtstate');
const scorePath = resolve(stage, 'stream-A.bbtscore');
const errorPath = resolve(stage, 'stream-A.bbterror');
const mappedSize = 64 + 1920 * 1080 * 4 * 3;

try {
  await mkdir(resolve(stage, 'bbt'), { recursive: true });
  await copyFile(resolve(root, 'tests/renderer-harness/main.lua'), resolve(stage, 'main.lua'));
  await copyFile(resolve(root, 'mod/shared/bbt/renderer.lua'), resolve(stage, 'bbt/renderer.lua'));
  await writeFile(statePath, Buffer.alloc(32));
  const score = Buffer.alloc(48);
  score.writeUInt32LE(1, 0);
  score.writeUInt32LE(1, 4);
  score.writeFloatLE(97.75, 8);
  score.writeFloatLE(-10.25, 12);
  for (const [offset, value] of [
    [16, 97],
    [20, 2],
    [24, 1],
    [28, 0],
    [32, 75],
    [36, 100],
    [40, 100],
    [44, 2],
  ])
    score.writeUInt32LE(value, offset);
  await writeFile(scorePath, score);
  const header = Buffer.alloc(64);
  header.write('BBTFRAME', 0, 'ascii');
  header.writeUInt32LE(2, 8);
  header.writeUInt32LE(320, 12);
  header.writeUInt32LE(180, 16);
  header.writeUInt32LE(320 * 4, 20);
  header.writeUInt32LE(3, 24);
  header.writeBigUInt64LE(BigInt(320 * 180 * 4), 40);
  await writeFile(framePath, header);
  const frameFile = await open(framePath, 'r+');
  await frameFile.truncate(mappedSize);
  await frameFile.close();

  // Reuse the ignored fused LÖVE fixture but strip its appended dashboard game
  // so this tracked harness is the only code the child executes.
  const fused = await readFile(resolve(fixture, 'BBTDashboardQA.exe'));
  const archiveOffset = fused.indexOf(Buffer.from([0x50, 0x4b, 0x03, 0x04]));
  assert.ok(archiveOffset > 0, 'could not locate the fused LÖVE archive');
  const executable = resolve(stage, 'renderer-qa.exe');
  await writeFile(executable, fused.subarray(0, archiveOffset));

  const child = spawn(executable, [stage], {
    windowsHide: true,
    cwd: stage,
    env: {
      ...process.env,
      PATH: `${fixture};${process.env.PATH}`,
      BBT_RENDERER_FRAME_PATH: framePath,
      BBT_RENDERER_ERROR_PATH: errorPath,
      BBT_RENDERER_MODE: mode,
      BBT_RENDERER_WIDTH: '320',
      BBT_RENDERER_HEIGHT: '180',
      BBT_RENDERER_FPS: '60',
      BBT_RENDERER_AUDIO: '0',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (value) => (stdout += value));
  child.stderr.on('data', (value) => (stderr += value));
  const exitCode = await new Promise((resolveExit, reject) => {
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error('renderer visual harness exceeded its 30 second timeout'));
    }, 30_000);
    child.on('exit', (code) => {
      clearTimeout(timer);
      resolveExit(code);
    });
    child.on('error', reject);
  });
  if (exitCode !== 0) {
    const rendererError = await readFile(errorPath, 'utf8').catch(() => '');
    throw new Error(
      `renderer visual harness failed (${exitCode})\n${stdout}\n${stderr}\n${rendererError}`,
    );
  }
  const rendererError = await readFile(errorPath, 'utf8').catch(() => '');
  assert.equal(rendererError, '', `renderer reported a capture error: ${rendererError}`);

  const frame = await readFile(framePath);
  assert.equal(frame.subarray(0, 8).toString('ascii'), 'BBTFRAME');
  assert.equal(frame.readUInt32LE(8), 2);
  const width = frame.readUInt32LE(12);
  const height = frame.readUInt32LE(16);
  const stride = frame.readUInt32LE(20);
  const frameCount = frame.readUInt32LE(24);
  const sequence = Number(frame.readBigUInt64LE(32));
  const frameSize = Number(frame.readBigUInt64LE(40));
  const dropped = Number(frame.readBigUInt64LE(48));
  assert.deepEqual(
    { width, height, stride, frameCount },
    {
      width: 320,
      height: 180,
      stride: 1280,
      frameCount: 3,
    },
  );
  assert.ok(
    sequence >= 30,
    `renderer published too few frames: ${sequence}\nstdout: ${stdout}\nstderr: ${stderr}`,
  );
  assert.ok(
    dropped <= Math.max(2, Math.floor(sequence * 0.1)),
    `renderer dropped ${dropped} frames for ${sequence} outputs`,
  );

  const offset = 64 + (sequence % frameCount) * frameSize;
  const center = offset + (Math.floor(height / 2) * width + Math.floor(width / 2)) * 4;
  const centerPixel = [...frame.subarray(center, center + 4)];
  const expectedCenter = mode === 'full' ? [153, 102, 51, 255] : [0, 255, 0, 255];
  assert.deepEqual(
    centerPixel,
    expectedCenter,
    mode === 'full'
      ? 'Full mode omitted the shaded player view or its final screen-space effect'
      : 'Clean mode omitted the palette/accessibility conversion of its base canvas',
  );
  const leftPixel = [...frame.subarray(offset, offset + 4)];
  assert.deepEqual(leftPixel, [0, 0, 0, 255], 'aspect-ratio pillarbox must remain opaque black');

  console.log(`Verified ${mode} OBS frame at 320x180 (${sequence} published, ${dropped} dropped).`);
} finally {
  await rm(stage, { recursive: true, force: true });
}
