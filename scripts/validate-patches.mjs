import { readFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const fixture = JSON.parse(
  await readFile(resolve(root, 'mod/fixtures/patch-signatures.json'), 'utf8'),
);
const hooks = await readFile(resolve(root, 'mod/shared/lovely/hooks.toml'), 'utf8');
const missing = fixture.patchPatterns.filter(
  (pattern) => !hooks.includes(`pattern = ${JSON.stringify(pattern)}`),
);
if (missing.length)
  throw new Error(`Lovely patch manifest is missing fixture signatures: ${missing.join(', ')}`);

const reference = resolve(root, '.reference/Beatblock/packed');
const blocks = hooks.match(/\[\[patches\]\][\s\S]*?(?=\[\[patches\]\]|$)/g) ?? [];
let sourceValidated = 0;
for (const block of blocks) {
  const targetMatch = block.match(/^target = (".*")$/m);
  const patternMatch = block.match(/^pattern = (".*")$/m);
  if (!targetMatch || !patternMatch) continue;
  const target = JSON.parse(targetMatch[1]);
  const pattern = JSON.parse(patternMatch[1]);
  if (!target.startsWith('states/')) continue;
  const source = execFileSync(
    'tar',
    ['-xOf', resolve(reference, 'states.zip'), target.slice('states/'.length)],
    { encoding: 'utf8' },
  );
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
  `Validated ${sourceValidated} Lovely source signatures and ${fixture.gameManagerHooks.length} GameManager hooks against the pinned reference.`,
);
