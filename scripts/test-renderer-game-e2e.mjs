import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const game = resolve(process.env.BBT_GAME_FIXTURE ?? 'E:/beatblock-online/.test/Beatblock');
const executable = resolve(game, 'Beatblock.exe');
const stage = await mkdtemp(resolve(tmpdir(), 'bbt-renderer-game-'));
const mods = resolve(stage, 'Mods');
const profile = resolve(stage, 'profile');
const chart = resolve(stage, 'tutorial');
const framePath = resolve(stage, 'stream-A.bbtframe');
const statePath = resolve(stage, 'stream-A.bbtstate');
const errorPath = resolve(stage, 'stream-A.bbterror');
const width = 320;
const height = 180;
const frameSize = width * height * 4;
const targetFrames = 480;
const mappedSize = 64 + 1920 * 1080 * 4 * 3;
let child;
let updater;
let stateFile;

try {
  await mkdir(mods, { recursive: true });
  await mkdir(profile, { recursive: true });
  await mkdir(chart, { recursive: true });
  await cp(resolve(root, 'mod/standalone'), resolve(mods, 'BeatblockOnlineRenderer'), {
    recursive: true,
  });
  // Production renderer paths point at an installed chart directory. Unpack
  // the game's tutorial into the isolated stage so this exercises that exact
  // contract instead of relying on Beatblock's menu-only virtual pack paths.
  const extracted = spawnSync(
    'tar',
    ['-xf', resolve(game, 'packed/levels/Finished levels/tutorial.zip'), '-C', chart],
    { encoding: 'utf8', windowsHide: true },
  );
  assert.equal(
    extracted.status,
    0,
    `could not unpack tutorial chart\n${extracted.stdout}\n${extracted.stderr}`,
  );

  const header = Buffer.alloc(64);
  header.write('BBTFRAME', 0, 'ascii');
  header.writeUInt32LE(2, 8);
  header.writeUInt32LE(width, 12);
  header.writeUInt32LE(height, 16);
  header.writeUInt32LE(width * 4, 20);
  header.writeUInt32LE(3, 24);
  header.writeBigUInt64LE(BigInt(frameSize), 40);
  await writeFile(framePath, header);
  const frameFile = await open(framePath, 'r+');
  await frameFile.truncate(mappedSize);
  await frameFile.close();
  await writeFile(statePath, Buffer.alloc(32));
  stateFile = await open(statePath, 'r+');

  child = spawn(executable, [], {
    windowsHide: true,
    cwd: game,
    env: {
      ...process.env,
      PATH: `${game};${process.env.PATH}`,
      SteamAppId: '3045200',
      SteamGameId: '3045200',
      APPDATA: profile,
      LOVELY_MOD_DIR: mods,
      BBT_RENDERER_STREAM: 'A',
      BBT_RENDERER_MODE: 'full',
      BBT_RENDERER_FRAME_PATH: framePath,
      BBT_RENDERER_ERROR_PATH: errorPath,
      BBT_RENDERER_WIDTH: String(width),
      BBT_RENDERER_HEIGHT: String(height),
      BBT_RENDERER_FPS: '60',
      BBT_RENDERER_AUDIO: '0',
      BBT_RENDERER_CHART: chart,
      BBT_RENDERER_VARIANT: 'easy',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (value) => (stdout += value));
  child.stderr.on('data', (value) => (stderr += value));
  let exited = false;
  let exitCode;
  child.on('exit', (code) => {
    exited = true;
    exitCode = code;
  });

  const startedAt = performance.now();
  let sequence = 0;
  updater = (async () => {
    while (!exited) {
      const elapsed = (performance.now() - startedAt) / 1000;
      const input = Buffer.alloc(32);
      input[0] = 3;
      input.writeFloatLE(Number.NaN, 12);
      input.writeFloatLE(0, 16);
      input.writeFloatLE(-8 + elapsed * 2, 20);
      input.writeFloatLE((elapsed * 45) % 360, 24);
      // Playing + capture enabled. The first commit also resets hidden motion.
      input.writeUInt16LE(1 | (1 << 4) | (sequence === 0 ? 1 << 5 : 0), 30);
      sequence = (sequence + 1) >>> 0 || 1;
      await stateFile.write(input, 0, 8, 0);
      await stateFile.write(input, 12, 20, 12);
      const commit = Buffer.alloc(4);
      commit.writeUInt32LE(sequence);
      await stateFile.write(commit, 0, 4, 8);
      await new Promise((resolveWait) => setTimeout(resolveWait, 16));
    }
  })();

  let published = 0;
  const deadline = performance.now() + 20_000;
  while (!exited && performance.now() < deadline) {
    await new Promise((resolveWait) => setTimeout(resolveWait, 250));
    const frame = await readFile(framePath);
    published = Number(frame.readBigUInt64LE(32));
    if (published >= targetFrames) break;
  }
  if (exited) {
    const rendererError = await readFile(errorPath, 'utf8').catch(() => '');
    throw new Error(
      `Beatblock renderer exited early (${exitCode})\n${stdout}\n${stderr}\n${rendererError}`,
    );
  }
  if (published < targetFrames) {
    const rendererError = await readFile(errorPath, 'utf8').catch(() => '');
    throw new Error(
      `Beatblock renderer published only ${published} frames\n${stdout}\n${stderr}\n${rendererError}`,
    );
  }

  const frame = await readFile(framePath);
  const frameCount = frame.readUInt32LE(24);
  const offset = 64 + (published % frameCount) * frameSize;
  const rgba = frame.subarray(offset, offset + frameSize);
  const colors = new Set();
  let brightness = 0;
  let darkPixels = 0;
  let nonWhitePixels = 0;
  for (let pixel = 0; pixel < rgba.length; pixel += 4) {
    const sum = rgba[pixel] + rgba[pixel + 1] + rgba[pixel + 2];
    if (sum < 120) darkPixels += 1;
    if (sum < 735) nonWhitePixels += 1;
  }
  for (let y = 0; y < height; y += 6) {
    for (let x = 0; x < width; x += 6) {
      const pixel = (y * width + x) * 4;
      colors.add(rgba.subarray(pixel, pixel + 4).toString('hex'));
      brightness += rgba[pixel] + rgba[pixel + 1] + rgba[pixel + 2];
    }
  }
  const report = resolve(root, 'reports/renderer-game-e2e.bmp');
  await mkdir(resolve(root, 'reports'), { recursive: true });
  await writeFile(report, rgbaToBmp(rgba, width, height));
  // Beatblock's tutorial intentionally uses a three-color palette. Pixel
  // coverage catches an empty white/black frame without requiring charts to
  // add colors that are not in their original art direction.
  assert.ok(colors.size >= 3, `physical renderer frame is visually empty (${colors.size} colors)`);
  assert.ok(
    darkPixels >= 500,
    `physical renderer lacks chart/paddle detail (${darkPixels} dark pixels)`,
  );
  assert.ok(
    nonWhitePixels >= 1_000,
    `physical renderer lacks its chart background (${nonWhitePixels} non-white pixels)`,
  );
  assert.ok(brightness > 10_000, 'physical renderer frame is black');
  console.log(
    `Verified physical Beatblock renderer (${published} frames, ${colors.size} sampled colors): ${report}`,
  );
} finally {
  if (child && child.exitCode === null) child.kill();
  if (updater) await updater.catch(() => {});
  if (stateFile) await stateFile.close().catch(() => {});
  await rm(stage, { recursive: true, force: true });
}

function rgbaToBmp(rgba, imageWidth, imageHeight) {
  const rowBytes = imageWidth * 4;
  const pixels = Buffer.alloc(rowBytes * imageHeight);
  for (let y = 0; y < imageHeight; y += 1) {
    const targetY = imageHeight - 1 - y;
    for (let x = 0; x < imageWidth; x += 1) {
      const source = (y * imageWidth + x) * 4;
      const target = targetY * rowBytes + x * 4;
      pixels[target] = rgba[source + 2];
      pixels[target + 1] = rgba[source + 1];
      pixels[target + 2] = rgba[source];
      pixels[target + 3] = rgba[source + 3];
    }
  }
  const header = Buffer.alloc(54);
  header.write('BM', 0, 'ascii');
  header.writeUInt32LE(header.length + pixels.length, 2);
  header.writeUInt32LE(header.length, 10);
  header.writeUInt32LE(40, 14);
  header.writeInt32LE(imageWidth, 18);
  header.writeInt32LE(imageHeight, 22);
  header.writeUInt16LE(1, 26);
  header.writeUInt16LE(32, 28);
  header.writeUInt32LE(pixels.length, 34);
  return Buffer.concat([header, pixels]);
}
