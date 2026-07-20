import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { basename, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const fixture = resolve(process.env.BBT_UI_FIXTURE ?? 'E:/beatblock-online/.test/ui-harness');
const baselines = resolve(root, 'tests/ui-baselines');
// A reviewer may have reports/ui open in an image viewer. Allow a fresh
// destination so verification never needs to close that application or
// overwrite a file while Windows still has it mapped.
const reports = resolve(process.env.BBT_UI_REPORTS ?? resolve(root, 'reports/ui'));
const python = process.env.BBT_UI_PYTHON ?? 'python';
const update = process.argv.includes('--update');
const stage = await mkdtemp(resolve(tmpdir(), 'bbt-ui-'));
const output = resolve(stage, 'captures');
await mkdir(resolve(stage, 'bbt'), { recursive: true });
await mkdir(output, { recursive: true });
await mkdir(reports, { recursive: true });
await cp(resolve(root, 'tests/ui-harness/main.lua'), resolve(stage, 'main.lua'));
for (const file of ['dashboard_model.lua', 'dashboard_components.lua', 'online_state.lua']) {
  await cp(resolve(root, 'mod/shared/bbt', file), resolve(stage, 'bbt', file));
}

// The ignored QA executable is a fused copy of LÖVE. Strip its appended game
// archive into the temporary stage so the tracked harness is the only source.
const fused = await readFile(resolve(fixture, 'BBTDashboardQA.exe'));
const archiveOffset = fused.indexOf(Buffer.from([0x50, 0x4b, 0x03, 0x04]));
if (archiveOffset < 1) throw new Error('Could not locate the fused LÖVE archive');
const executable = resolve(stage, 'love-ui-qa.exe');
await writeFile(executable, fused.subarray(0, archiveOffset));
const child = spawn(executable, [stage], {
  windowsHide: true,
  cwd: stage,
  env: {
    ...process.env,
    PATH: `${fixture};${process.env.PATH}`,
    BBT_UI_FIXTURE: fixture,
    BBT_UI_OUTPUT: output,
    BBT_UI_AUTORUN: '1',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let stdout = '',
  stderr = '';
child.stdout.on('data', (value) => (stdout += value));
child.stderr.on('data', (value) => (stderr += value));
const exitCode = await new Promise((resolveExit, reject) => {
  const timer = setTimeout(() => {
    child.kill();
    reject(new Error('UI harness exceeded its 45 second timeout'));
  }, 45_000);
  child.on('exit', (code) => {
    clearTimeout(timer);
    resolveExit(code);
  });
  child.on('error', reject);
});
if (exitCode !== 0) throw new Error(`UI harness failed (${exitCode})\n${stdout}\n${stderr}`);

const audit = await readFile(resolve(output, 'layout-audit.txt'), 'utf8');
if (audit.split(/\r?\n/).some((line) => /:\d+$/.test(line) && !line.endsWith(':0'))) {
  throw new Error(`Layout audit failed:\n${audit}`);
}
const captures = (await readdir(output)).filter((file) => file.endsWith('.png')).sort();
if (captures.length < 26) throw new Error(`Expected 26 UI scenarios, captured ${captures.length}`);
if (update) await mkdir(baselines, { recursive: true });

for (const file of captures) {
  const actual = resolve(output, file);
  const baseline = resolve(baselines, file);
  const report = resolve(reports, file);
  const diffReport = resolve(reports, file.replace('.png', '.diff.png'));
  assertPngDimensions(actual, await readFile(actual), 600, 360);
  await cp(actual, report);
  // Nearest-neighbor review artifacts preserve the source 600x360 pixels.
  await run(python, [
    '-c',
    'from PIL import Image; import sys; im=Image.open(sys.argv[1]); im.resize((1200,720),Image.Resampling.NEAREST).save(sys.argv[2])',
    actual,
    resolve(reports, file.replace('.png', '@2x.png')),
  ]);
  if (update) {
    await cp(actual, baseline);
    await rm(diffReport, { force: true });
  } else {
    assertPngDimensions(baseline, await readFile(baseline), 600, 360);
    await run(python, [
      resolve(root, 'scripts/ui-image-compare.py'),
      actual,
      baseline,
      diffReport,
      '--threshold',
      '0.1',
      '--max-changed-percent',
      '0.05',
    ]);
    // Passing comparisons should not leave stale review artifacts that look
    // like unresolved visual regressions.
    await rm(diffReport, { force: true });
  }
}
await rm(stage, { recursive: true, force: true });
console.log(`${update ? 'Updated' : 'Verified'} ${captures.length} deterministic UI screenshots.`);

function run(command, args) {
  return new Promise((resolveRun, reject) => {
    const process = spawn(command, args, { windowsHide: true, stdio: 'inherit' });
    process.on('exit', (code) =>
      code === 0 ? resolveRun() : reject(new Error(`${basename(command)} exited ${code}`)),
    );
    process.on('error', reject);
  });
}

function assertPngDimensions(path, bytes, width, height) {
  if (
    bytes.length < 24 ||
    bytes.subarray(1, 4).toString('ascii') !== 'PNG' ||
    bytes.readUInt32BE(16) !== width ||
    bytes.readUInt32BE(20) !== height
  ) {
    const actual =
      bytes.length >= 24 ? `${bytes.readUInt32BE(16)}x${bytes.readUInt32BE(20)}` : 'not a PNG';
    throw new Error(`${path} is ${actual}; deterministic UI captures must be ${width}x${height}`);
  }
}
