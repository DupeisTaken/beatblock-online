import { readFile } from 'node:fs/promises';
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

// Patch acceptance is intentionally tied to the isolated `.test` game copy;
// release validation must never inspect or launch the user's Steam install.
const reference = resolve(root, '.test/Beatblock/packed');
const gameExecutable = resolve(root, '.test/Beatblock/Beatblock.exe');
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
