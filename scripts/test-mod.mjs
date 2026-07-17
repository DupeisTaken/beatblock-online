import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { listZipEntries } from './verify-release.mjs';

const root = resolve(import.meta.dirname, '..');
const read = (path) => readFile(resolve(root, path), 'utf8');
const hash = (value) => createHash('sha256').update(value).digest('hex');
const [core, dashboard, online, ipc, renderer, hooks, commands, readme, obsPlugin] =
  await Promise.all([
    read('mod/shared/bbt/core.lua'),
    read('mod/shared/bbt/dashboard_model.lua'),
    read('mod/shared/bbt/online_state.lua'),
    read('mod/shared/bbt/ipc_thread.lua'),
    read('mod/shared/bbt/renderer.lua'),
    read('mod/shared/lovely/hooks.toml'),
    read('companion/src/game_commands.rs'),
    read('README.md'),
    read('obs-plugin/src/plugin.c'),
  ]);
const bootstrap = await read('mod/standalone/lovely/bootstrap.toml');
if (!bootstrap.includes('{{lovely_hack:patch_dir}}'))
  throw new Error("Standalone bootstrap must use Lovely's supported patch_dir placeholder");
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
  [ipc, 'CreateProcessA'],
  [ipc, 'beatblock-together-v2'],
  [ipc, 'runtime.disconnected'],
  [ipc, 'PeekNamedPipe'],
  [ipc, 'ERROR_PIPE_BUSY'],
  [ipc, 'math.min(8, 2 ^ math.min(launchAttempts - 1, 3))'],
  [core, 'pendingRequestDeadlineMs'],
  [core, "message.type == 'runtime.disconnected'"],
  [core, 'CLIENT_INSTANCE_ID'],
  [core, "BBT.send('client.ping'"],
  [core, "message.type == 'runtime.heartbeat'"],
  [core, 'function BBT.cancelChartSelection(selector)'],
  [core, 'BBT.context.sessionId = message.payload.sessionId'],
  [core, "cs.topDirectory = 'levels/Songwheel/'"],
  [core, 'local officialSelection = BBT.selectingOfficialChart'],
  [core, 'official = officialSelection'],
  [core, 'local function chartPreloadReady(levelData, soundData)'],
  [core, 'previous.menuMusicManager:stop()'],
  [core, 'local renderInterval = inGame and 1 / 60 or 1 / 5'],
  [renderer, 'readbackPending = {false,false}'],
  [renderer, 'readbackRequests = {nil,nil}'],
  [renderer, 'readbackTickets = {0,0}'],
  [renderer, 'function Renderer.shutdown()'],
  [renderer, 'local success, request = pcall(love.graphics.readbackTextureAsync, output)'],
  [renderer, 'cLevel = chart'],
  [renderer, "chart = chart .. '/'"],
  [renderer, "pcall(dpf.loadJson, chart .. 'manifest.json')"],
  [renderer, "if type(value) == 'number' then savedata.options.audio[key] = 0 end"],
  [renderer, 'cs:init(chart, variantInfo, nil, {})'],
  [renderer, 'sdfunc.save = function() end'],
  [renderer, 'Renderer.originalErrorHandler = love.errorhandler'],
  [renderer, "reportError('renderer crashed:\\n'"],
  [renderer, 'function Renderer.captureSafe(source)'],
  [renderer, 'Renderer.frames.pointer + 32'],
  [online, 'Room password must contain 4-128 characters.'],
  [online, "love.graphics.printf(BBT.lastError,74,239,452,'center')"],
]) {
  if (!contract[0].includes(contract[1]))
    throw new Error(`Lazy runtime contract is missing ${contract[1]}`);
}
if (ipc.includes('launchCount >= 2'))
  throw new Error('IPC runtime recovery still stops permanently after two launch attempts');
if (!hooks.includes('name = "bbt.dashboard_model"'))
  throw new Error('Lovely does not register the adaptive dashboard model before main.lua');
if (!hooks.includes('cs = bs.load(project.initState)\\n\\tcs:init()'))
  throw new Error('Renderer startup hook is not qualified against the editor init path');
if (hooks.includes('states/AtomMap.lua'))
  throw new Error('Online chart selection still patches the brittle Atom Map state');
if (renderer.includes('readbackTextureAsync, output, function'))
  throw new Error('Renderer still uses the obsolete callback-style LÖVE readback API');
if (!obsPlugin.includes('read_committed_sequence(header)'))
  throw new Error('OBS source does not confirm read-only sequence snapshots around its frame copy');
if (obsPlugin.includes('InterlockedCompareExchange64'))
  throw new Error('OBS source performs a write primitive against its FILE_MAP_READ frame view');
if (!hooks.includes('BBTRenderer.captureSafe(self.canv)'))
  throw new Error('Renderer capture hook can still crash the Beatblock child');
if (!hooks.includes('BBTRenderer.shouldHold() and not self.startPending then return end'))
  throw new Error('Renderer hold can block Beatblock before chart preloading completes');
const officialSelect = core.slice(
  core.indexOf('function BBT.openOfficialSelect(mode)'),
  core.indexOf('local function returnFromChartSelector'),
);
for (const contract of [
  "cs = bs.load('SongSelect')",
  "cs.topDirectory = 'levels/Songwheel/'",
  'BBT.selectingOfficialChart = true',
]) {
  if (!officialSelect.includes(contract))
    throw new Error(`Official Freeplay selection is missing ${contract}`);
}
if (officialSelect.includes("bs.load('AtomMap')"))
  throw new Error('Official selection still loads Atom Map instead of Freeplay');
const scheduledLaunch = core.slice(
  core.indexOf('function BBT.maybeLaunchScheduledChart()'),
  core.indexOf('function BBT.init(distribution'),
);
for (const contract of [
  'previous.source.stop',
  'previous.menuMusicManager:clearOnBeatHooks()',
  'previous.menuMusicManager:stop()',
]) {
  if (!scheduledLaunch.includes(contract))
    throw new Error(`Scheduled launch audio shutdown is missing ${contract}`);
}
if (
  scheduledLaunch.indexOf('previous.menuMusicManager:stop()') >
  scheduledLaunch.indexOf('cs:init(BBT.localChart')
)
  throw new Error('Menu music is stopped after Game initialization');
if (!hooks.includes('loc.json.bbtOnline') || !hooks.includes('sprites.menu.play'))
  throw new Error('Online menu entry must provide a localized label and native icon');
if (
  core.includes("BBT.send('client.hello'") &&
  core.indexOf("BBT.send('client.hello'") < core.indexOf('function BBT.startOnlineRuntime()')
)
  throw new Error('Runtime hello is sent before entering Online');
if (`${core}\n${online}`.includes('manager.open_request'))
  throw new Error('Obsolete visible Manager command remains');

for (const contrastContract of [
  'muted={1,1,1,.68}',
  'dimBlack={0,0,0,.55}',
  'setc(available and C.black or C.dimBlack)',
  'setc(active and C.black or C.white)',
]) {
  if (!online.includes(contrastContract))
    throw new Error(`Online font contrast contract is missing ${contrastContract}`);
}
if (online.includes('local PAGES ='))
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
  if (!readme.includes(readmeSection)) throw new Error(`README is missing ${readmeSection}`);
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
  'SELECT FREEPLAY CHART',
  'SELECT CUSTOM',
  'LOCATE MATCHING CHART',
  'READY',
  'START RACE',
  'FORCE START',
  'SPECTATE + OBS',
  'MATCH HISTORY',
  'SETTINGS + DIAGNOSTICS',
]) {
  if (!`${online}\n${dashboard}`.includes(capability))
    throw new Error(`Online dashboard is missing ${capability}`);
}
for (const interaction of [
  "pressed('select')",
  'mouse.pressed==1',
  'love.keyboard.isDown',
  'ROOM ADDRESS',
]) {
  if (!online.includes(interaction))
    throw new Error(`Online state is missing interaction contract: ${interaction}`);
}
for (const signature of [
  'function st:playLevel(filename, variant)',
  'function st:quitToMenu()',
  'BBT.cancelChartSelection(self)',
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
  'local function returnFromChartSelector(selector)',
  'music:forceUnmute()',
  'cs.menuMusicManager = music',
]) {
  if (!core.includes(handoff))
    throw new Error(`Chart-selection music handoff is missing ${handoff}`);
}
if (core.includes('expectedMaxHits = 1'))
  throw new Error('Chart verification still contains the placeholder max-hit count');

for (const distribution of ['standalone', 'beatblock-plus']) {
  for (const file of [
    'core.lua',
    'dashboard_model.lua',
    'online_state.lua',
    'ipc_thread.lua',
    'renderer.lua',
  ]) {
    const shared = await read(`mod/shared/bbt/${file}`);
    const packaged = await read(`mod/${distribution}/bbt/${file}`);
    if (hash(shared) !== hash(packaged))
      throw new Error(`${distribution}/${file} was not generated from the shared core`);
  }
  const archive = resolve(
    root,
    `mod/releases/beatblock-together-${distribution}-0.3.0-alpha.1.zip`,
  );
  const entries = new Set(
    listZipEntries(await readFile(archive), `${distribution} release ZIP`).map((entry) =>
      entry.replaceAll('\\', '/'),
    ),
  );
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
    if (!entries.has(prefix + required))
      throw new Error(`${distribution} release ZIP is missing ${required}`);
  }
}

console.log(
  `Validated ${requiredCommands.length} in-game commands, chart/start/HUD hooks, and both generated release ZIPs.`,
);
