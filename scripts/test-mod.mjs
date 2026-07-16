import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const read = (path) => readFile(resolve(root, path), 'utf8');
const hash = (value) => createHash('sha256').update(value).digest('hex');
const [core, dashboard, online, ipc, hooks, commands, readme] = await Promise.all([
  read('mod/shared/bbt/core.lua'),
  read('mod/shared/bbt/dashboard_model.lua'),
  read('mod/shared/bbt/online_state.lua'),
  read('mod/shared/bbt/ipc_thread.lua'),
  read('mod/shared/lovely/hooks.toml'),
  read('companion/src/game_commands.rs'),
  read('README.md'),
]);
const bootstrap = await read('mod/standalone/lovely/bootstrap.toml');
if (!bootstrap.includes('{{lovely_hack:patch_dir}}'))
  throw new Error('Standalone bootstrap must use Lovely\'s supported patch_dir placeholder');
if (bootstrap.includes('{{lovely_hack::patch_dir}}'))
  throw new Error('Standalone bootstrap contains the invalid double-colon patch_dir placeholder');

for (const contract of [
  [core, 'protocolVersion = 2'],
  [core, 'function BBT.startOnlineRuntime()'],
  [core, 'function BBT.exitOnline()'],
  [core, "gsub('[\\r\\n]', '')"],
  [online, 'BBT.startOnlineRuntime()'],
  [core, "BBT.command('runtime.session_end'"],
  [ipc, 'runtime-path.txt'],
  [ipc, 'WinExec'],
  [ipc, 'beatblock-together-v2'],
]) {
  if (!contract[0].includes(contract[1])) throw new Error(`Lazy runtime contract is missing ${contract[1]}`);
}
if (!hooks.includes('name = "bbt.dashboard_model"'))
  throw new Error('Lovely does not register the adaptive dashboard model before main.lua');
if (core.includes("BBT.send('client.hello'") && core.indexOf("BBT.send('client.hello'") < core.indexOf('function BBT.startOnlineRuntime()'))
  throw new Error('Runtime hello is sent before entering Online');
if (`${core}\n${online}`.includes('manager.open_request')) throw new Error('Obsolete visible Manager command remains');

for (const contrastContract of [
  'muted={1,1,1,.68}',
  'dimBlack={0,0,0,.55}',
  'setc(available and C.black or C.dimBlack)',
  'setc(active and C.black or C.white)',
]) {
  if (!online.includes(contrastContract))
    throw new Error(`Online font contrast contract is missing ${contrastContract}`);
}
if (online.includes("local PAGES ="))
  throw new Error('Online still uses the obsolete six-page tab bar');
for (const dashboardContract of [
  "require('bbt.dashboard_model')",
  "{id='setlist',label='SETLIST'}",
  'for row=1,8 do',
  'self.rosterOffset',
  "self.sideMenu='participant'",
  'local function requestConfirm',
  'local function drawHelp',
  'local function drawOverlay',
  "self.focusZone='primary'",
]) {
  if (!online.includes(dashboardContract))
    throw new Error(`Concentrated dashboard contract is missing ${dashboardContract}`);
}
for (const modelContract of [
  'function Dashboard.phase(context)',
  'function Dashboard.summary(context)',
  'function Dashboard.primary(context)',
  "return action('select_chart','SELECT CHART'",
  "return action('locate_chart','LOCATE MATCHING CHART'",
  "return action('start_race','START RACE'",
  "return action('advance_set','ADVANCE SETLIST'",
  'function Dashboard.help(context, overlay)',
]) {
  if (!dashboard.includes(modelContract))
    throw new Error(`Adaptive dashboard model is missing ${modelContract}`);
}
for (const obsoleteCopy of [
  'No accounts or cloud server',
  'HOST OR JOIN FIRST',
  'Setlists belong to a room',
]) {
  if (online.includes(obsoleteCopy))
    throw new Error(`Online state still contains obsolete exclusion copy: ${obsoleteCopy}`);
}
for (const readmeSection of [
  '## Player quick start',
  '## Developer commands',
  '## Detailed documentation',
  'docs/injection.md',
  'docs/benchmarking.md',
]) {
  if (!readme.includes(readmeSection))
    throw new Error(`README is missing ${readmeSection}`);
}

const requiredCommands = [
  'room.host_request',
  'room.join_request',
  'room.chart_select_request',
  'room.chart_verify_request',
  'room.ready_request',
  'room.start_request',
  'room.leave_request',
  'room.close_request',
  'room.official_chart_select',
  'room.admission_set',
  'room.role_set',
  'room.kick',
  'setlist.advance',
  'renderer.configure',
  'history.list',
  'settings.update',
  'diagnostics.get',
  'runtime.session_end',
];
for (const command of requiredCommands) {
  if (!core.includes(command) && !online.includes(command))
    throw new Error(`The in-game mod does not emit ${command}`);
  if (!commands.includes(`"${command}"`))
    throw new Error(`The companion does not handle ${command}`);
}

for (const capability of [
  'HOST A ROOM',
  'JOIN ROOM',
  'JOIN AS SPECTATOR',
  'PLAY ONLINE',
  'SELECT CUSTOM',
  'LOCATE MATCHING CHART',
  'READY',
  'START RACE',
  'FORCE START',
  'SPECTATE + OBS',
  'MATCH HISTORY',
  'SETTINGS + DIAGNOSTICS',
]) {
  if (!`${online}\n${dashboard}`.includes(capability)) throw new Error(`Online dashboard is missing ${capability}`);
}
for (const interaction of [
  "pressed('select')",
  'mouse.pressed==1',
  'love.keyboard.isDown',
  'HOST IP:PORT',
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
  'em.clear({self.menuMusicManager})',
  'self.menuMusicManager:update(dt)',
  'cs.menuMusicManager=music',
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
  for (const file of ['core.lua', 'dashboard_model.lua', 'online_state.lua', 'ipc_thread.lua', 'renderer.lua']) {
    const shared = await read(`mod/shared/bbt/${file}`);
    const packaged = await read(`mod/${distribution}/bbt/${file}`);
    if (hash(shared) !== hash(packaged))
      throw new Error(`${distribution}/${file} was not generated from the shared core`);
  }
  const archive = resolve(root, `mod/releases/beatblock-together-${distribution}-0.3.0-alpha.1.zip`);
  const entries = execFileSync('tar', ['-tf', archive], { encoding: 'utf8' });
  const prefix = 'BeatblockTogether/';
  const distributionFiles =
    distribution === 'standalone'
      ? ['lovely/bootstrap.toml', 'README.txt']
      : ['mod.json', 'main.lua', 'config.lua', 'states/Online.lua', 'README.txt'];
  for (const required of [
    'bbt/core.lua',
    'bbt/dashboard_model.lua',
    'bbt/online_state.lua',
    'bbt/renderer.lua',
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
