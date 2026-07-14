import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const read = (path) => readFile(resolve(root, path), 'utf8');
const hash = (value) => createHash('sha256').update(value).digest('hex');
const [core, online, hooks, commands] = await Promise.all([
  read('mod/shared/bbt/core.lua'),
  read('mod/shared/bbt/online_state.lua'),
  read('mod/shared/lovely/hooks.toml'),
  read('companion/src/game_commands.rs'),
]);

const requiredCommands = [
  'lobby.create_request',
  'lobby.join_request',
  'lobby.chart_select_request',
  'lobby.chart_verify_request',
  'lobby.ready_request',
  'lobby.start_request',
  'lobby.leave_request',
  'lobby.close_request',
];
for (const command of requiredCommands) {
  if (!core.includes(command) && !online.includes(command))
    throw new Error(`The in-game mod does not emit ${command}`);
  if (!commands.includes(`"${command}"`))
    throw new Error(`The companion does not handle ${command}`);
}

for (const capability of [
  'JOIN AS PLAYER',
  'JOIN AS SPECTATOR',
  'PRACTICE + TELEMETRY',
  'SELECT CUSTOM CHART',
  'LOCATE MATCHING CHART',
  'READY FOR RACE',
  'START SYNCHRONIZED RACE',
]) {
  if (!online.includes(capability)) throw new Error(`Online state is missing ${capability}`);
}
for (const interaction of [
  "pressed('select')",
  'mouse.pressed == 1',
  'love.keyboard.isDown',
  'disabledReason',
  'LOBBY CODE',
]) {
  if (!online.includes(interaction))
    throw new Error(`Online state is missing interaction contract: ${interaction}`);
}
for (const signature of [
  'function st:playLevel(filename, variant)',
  'if self.pauseTimer <= 0 then',
  'self.gm:endOnTopShader()',
]) {
  if (!hooks.includes(signature)) throw new Error(`Gameplay integration is missing ${signature}`);
}
for (const handoff of [
  'local bbtMenuMusicManager = self.menuMusicManager',
  'cs.menuMusicManager = bbtMenuMusicManager',
]) {
  if (!hooks.includes(handoff)) throw new Error(`Main-menu music handoff is missing ${handoff}`);
}
for (const handoff of [
  'em.clear({ self.menuMusicManager })',
  'self.menuMusicManager:update(dt)',
  'cs.menuMusicManager = menuMusicManager',
]) {
  if (!online.includes(handoff)) throw new Error(`Online music continuity is missing ${handoff}`);
}
for (const handoff of [
  'local menuMusicManager = selector.menuMusicManager',
  'menuMusicManager:forceUnmute()',
  'cs.menuMusicManager = menuMusicManager',
]) {
  if (!core.includes(handoff))
    throw new Error(`Chart-selection music handoff is missing ${handoff}`);
}
if (core.includes('expectedMaxHits = 1'))
  throw new Error('Chart verification still contains the placeholder max-hit count');

for (const distribution of ['standalone', 'beatblock-plus']) {
  for (const file of ['core.lua', 'online_state.lua', 'ipc_thread.lua']) {
    const shared = await read(`mod/shared/bbt/${file}`);
    const packaged = await read(`mod/${distribution}/bbt/${file}`);
    if (hash(shared) !== hash(packaged))
      throw new Error(`${distribution}/${file} was not generated from the shared core`);
  }
  const archive = resolve(
    root,
    `mod/releases/beatblock-together-${distribution}-0.1.0-alpha.1.zip`,
  );
  const entries = execFileSync('tar', ['-tf', archive], { encoding: 'utf8' });
  const prefix = 'BeatblockTogether/';
  const distributionFiles =
    distribution === 'standalone'
      ? ['lovely/bootstrap.toml', 'README.txt']
      : ['mod.json', 'main.lua', 'config.lua', 'states/Online.lua', 'README.txt'];
  for (const required of [
    'bbt/core.lua',
    'bbt/online_state.lua',
    'lovely/hooks.toml',
    ...distributionFiles,
  ]) {
    if (!entries.replaceAll('\\', '/').includes(prefix + required))
      throw new Error(`${distribution} release ZIP is missing ${required}`);
  }
}

console.log(
  `Validated ${requiredCommands.length} in-game commands, chart/start/HUD hooks, and both generated release ZIPs.`,
);
