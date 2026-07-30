import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const supportedArguments = new Set(['--source-only']);
const unsupportedArguments = process.argv
  .slice(2)
  .filter((argument) => !supportedArguments.has(argument));
if (unsupportedArguments.length) {
  throw new Error(`Unsupported patch-validation argument: ${unsupportedArguments.join(', ')}`);
}
const sourceOnly = process.argv.includes('--source-only');
const fixture = JSON.parse(
  await readFile(resolve(root, 'mod/fixtures/patch-signatures.json'), 'utf8'),
);
const hooks = await readFile(resolve(root, 'mod/shared/lovely/hooks.toml'), 'utf8');
const missing = fixture.patchPatterns.filter(
  (pattern) => !hooks.includes(`pattern = ${JSON.stringify(pattern)}`),
);
if (missing.length)
  throw new Error(`Lovely patch manifest is missing fixture signatures: ${missing.join(', ')}`);

// Hosted runners cannot legally or reliably carry the proprietary game archives.
// Source-only validation still verifies every pinned signature is present in the
// manifest; the default local path below additionally proves them against `.test`.
if (sourceOnly) {
  console.log(
    `Validated ${fixture.patchPatterns.length} Lovely patch manifest signatures (source-only).`,
  );
  process.exit(0);
}

// Patch acceptance is intentionally tied to an isolated game copy; release
// validation must never inspect or launch the user's Steam install. Worktrees
// can point at the retained fixture without duplicating proprietary archives.
const gameFixture = resolve(process.env.BBT_GAME_FIXTURE ?? resolve(root, '.test/Beatblock'));
const reference = resolve(gameFixture, 'packed');
const gameExecutable = resolve(gameFixture, 'Beatblock.exe');
const pinnedArtifacts = [
  {
    path: gameExecutable,
    label: 'Beatblock.exe',
    expected: fixture.reference.beatblockExeSha256,
  },
  {
    path: resolve(reference, 'obj.zip'),
    label: 'packed/obj.zip',
    expected: fixture.reference.objectArchiveSha256,
  },
  {
    path: resolve(reference, 'states.zip'),
    label: 'packed/states.zip',
    expected: fixture.reference.stateArchiveSha256,
  },
];
const sha256File = (path) =>
  new Promise((resolveHash, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(path);
    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolveHash(hash.digest('hex')));
  });
for (const artifact of pinnedArtifacts) {
  const actual = await sha256File(artifact.path);
  if (actual !== artifact.expected) {
    throw new Error(
      `Pinned fixture hash mismatch for ${artifact.label}: expected ${artifact.expected}, got ${actual}`,
    );
  }
}
const blocks = hooks.match(/\[\[patches\]\][\s\S]*?(?=\[\[patches\]\]|$)/g) ?? [];
let sourceValidated = 0;
for (const block of blocks) {
  const targetMatch = block.match(/^target = (".*")$/m);
  const patternMatch = block.match(/^pattern = (".*")$/m);
  if (!targetMatch || !patternMatch) continue;
  const target = JSON.parse(targetMatch[1]);
  const pattern = JSON.parse(patternMatch[1]);
  // Beatblock's fused executable carries main.lua while state sources live in
  // the packed state archive. Validate both so a post-shuv capture hook cannot
  // silently drift to an invalid main-loop signature.
  let source;
  if (target === 'main.lua') {
    source = execFileSync('tar', ['-xOf', gameExecutable, target], { encoding: 'utf8' });
  } else if (target.startsWith('states/')) {
    source = execFileSync(
      'tar',
      ['-xOf', resolve(reference, 'states.zip'), target.slice('states/'.length)],
      { encoding: 'utf8' },
    );
  } else if (target.startsWith('obj/')) {
    source = execFileSync(
      'tar',
      ['-xOf', resolve(reference, 'obj.zip'), target.slice('obj/'.length)],
      { encoding: 'utf8' },
    );
  } else {
    continue;
  }
  source = source.replace(/\r\n/g, '\n');
  if (!source.includes(pattern))
    throw new Error(
      `Lovely signature is absent from the pinned game source: ${target} :: ${pattern}`,
    );
  sourceValidated += 1;
}

const gameManager = execFileSync(
  'tar',
  ['-xOf', resolve(reference, 'obj.zip'), 'GameManager.lua'],
  { encoding: 'utf8' },
);
const missingManagerHooks = fixture.gameManagerHooks.filter(
  (pattern) => !gameManager.includes(pattern),
);
if (missingManagerHooks.length)
  throw new Error(
    `GameManager hooks are absent from the pinned source: ${missingManagerHooks.join(', ')}`,
  );
console.log(
  `Validated ${sourceValidated} Lovely source signatures and ${fixture.gameManagerHooks.length} GameManager hooks against the isolated .test build.`,
);
