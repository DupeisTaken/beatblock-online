import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

// `pnpm test:ui` cannot run in CI: it needs a legally-obtained Beatblock fixture
// that no runner has. That gap is how the harness drifted to 12pt/14pt fonts and
// stayed there long enough to record all 44 baselines against metrics the game
// never renders. These are deliberately static text checks so the parts of the
// contract that caused that drift are enforced on every pull request instead.
const root = resolve(import.meta.dirname, '..');
const read = (path) => readFile(resolve(root, path), 'utf8');

// Beatblock's own preload/fonts.lua, transcribed. Update this table only when
// the game changes, never to make a failing layout pass.
const GAME_FONTS = [
  { name: 'main', file: 'Axmolotl.ttf', size: 16 },
  { name: 'digitalDisco', file: 'DigitalDisco-Thin.ttf', size: 16 },
];

test('the UI harness builds the fonts Beatblock actually renders', async () => {
  const harness = await read('tests/ui-harness/main.lua');
  for (const { name, file, size } of GAME_FONTS) {
    const declaration = new RegExp(
      `${name}\\s*=\\s*externalFont\\('${file.replaceAll('.', '\\.')}'\\s*,\\s*(\\d+)\\)`,
    );
    const match = harness.match(declaration);
    assert.ok(match, `the harness must build ${name} from ${file}, because preload/fonts.lua does`);
    assert.equal(
      Number(match[1]),
      size,
      `harness ${name} is ${match[1]}pt but Beatblock renders it at ${size}pt; ` +
        'a smaller size here silently hides real text overflow',
    );
  }
});

test('Online reasserts the shared menu font its baselines were recorded at', async () => {
  const online = await read('mod/shared/bbt/online_state.lua');
  assert.match(
    online,
    /love\.graphics\.setFont\(fonts\.digitalDisco\)/,
    'Online must keep reasserting fonts.digitalDisco, which is what the harness mirrors',
  );
});

test('autorun captures park the pointer instead of sampling the OS cursor', async () => {
  const harness = await read('tests/ui-harness/main.lua');
  const start = harness.indexOf('function love.update');
  const end = harness.indexOf('function love.draw');
  assert.ok(start >= 0 && end > start, 'love.update must precede love.draw in the harness');
  assert.match(
    harness.slice(start, end),
    /if autorun then mouse\.rx=-\d+; mouse\.ry=-\d+/,
    'autorun must park the pointer off-canvas: Online moves focus onto the hovered ' +
      'control, so sampling the real cursor makes a "deterministic" capture depend on ' +
      'where the physical mouse happens to rest',
  );
});

test('the layout audit still reports the overflow classes it was blind to', async () => {
  const components = await read('mod/shared/bbt/dashboard_components.lua');
  for (const issue of [
    'text_wrap_overflow', // copy silently ellipsized past its budget
    'text_overlap', // two text blocks sharing a panel and colliding
    'text_behind_control', // copy painted over by an opaque control fill
    'text_outside_panel', // copy escaping the panel that owns it
  ]) {
    assert.ok(
      components.includes(issue),
      `dashboard_components must still raise ${issue}; removing it re-opens a ` +
        'bug class the pixel comparison cannot see',
    );
  }
});
