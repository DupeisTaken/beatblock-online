import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inflateSync } from 'node:zlib';
import { listZipEntries } from './verify-release.mjs';

// Decode the small checked-in RGBA icon with Node built-ins so the packaging
// gate can enforce contrast without adding an image dependency to CI.
function decodeRgbaPng(buffer) {
  if (!buffer.subarray(0, 8).equals(Buffer.from('\x89PNG\r\n\x1a\n', 'binary')))
    throw new Error('Online menu icon has an invalid PNG signature');
  let offset = 8;
  let width;
  let height;
  const compressed = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString('ascii', offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[9] !== 6)
        throw new Error('Online menu icon must use 8-bit RGBA pixels');
    } else if (type === 'IDAT') {
      compressed.push(data);
    } else if (type === 'IEND') {
      break;
    }
    offset += length + 12;
  }
  if (!width || !height || compressed.length === 0)
    throw new Error('Online menu icon is incomplete');

  const source = inflateSync(Buffer.concat(compressed));
  const stride = width * 4;
  const pixels = Buffer.alloc(stride * height);
  const paeth = (left, up, upperLeft) => {
    const prediction = left + up - upperLeft;
    const leftDistance = Math.abs(prediction - left);
    const upDistance = Math.abs(prediction - up);
    const upperLeftDistance = Math.abs(prediction - upperLeft);
    return leftDistance <= upDistance && leftDistance <= upperLeftDistance
      ? left
      : upDistance <= upperLeftDistance
        ? up
        : upperLeft;
  };
  let sourceOffset = 0;
  for (let y = 0; y < height; y += 1) {
    const filter = source[sourceOffset++];
    for (let x = 0; x < stride; x += 1) {
      const raw = source[sourceOffset++];
      const target = y * stride + x;
      const left = x >= 4 ? pixels[target - 4] : 0;
      const up = y > 0 ? pixels[target - stride] : 0;
      const upperLeft = x >= 4 && y > 0 ? pixels[target - stride - 4] : 0;
      const predictor =
        filter === 0
          ? 0
          : filter === 1
            ? left
            : filter === 2
              ? up
              : filter === 3
                ? Math.floor((left + up) / 2)
                : filter === 4
                  ? paeth(left, up, upperLeft)
                  : Number.NaN;
      if (Number.isNaN(predictor))
        throw new Error(`Online menu icon uses unknown PNG filter ${filter}`);
      pixels[target] = (raw + predictor) & 0xff;
    }
  }
  return { width, height, pixels };
}

const root = resolve(import.meta.dirname, '..');
const read = (path) => readFile(resolve(root, path), 'utf8');
const hash = (value) => createHash('sha256').update(value).digest('hex');
const [core, dashboard, components, online, ipc, renderer, hooks, commands, readme, obsPlugin] =
  await Promise.all([
    read('mod/shared/bbt/core.lua'),
    read('mod/shared/bbt/dashboard_model.lua'),
    read('mod/shared/bbt/dashboard_components.lua'),
    read('mod/shared/bbt/online_state.lua'),
    read('mod/shared/bbt/ipc_thread.lua'),
    read('mod/shared/bbt/renderer.lua'),
    read('mod/shared/lovely/hooks.toml'),
    read('companion/src/game_commands.rs'),
    read('README.md'),
    read('obs-plugin/src/plugin.c'),
  ]);
const onlineIcon = await readFile(resolve(root, 'mod/shared/assets/online.png'));
const decodedOnlineIcon = decodeRgbaPng(onlineIcon);
if (decodedOnlineIcon.width !== 72 || decodedOnlineIcon.height !== 72)
  throw new Error('Online menu icon must be a valid 72x72 PNG');
let blackPixels = 0;
let whitePixels = 0;
let transparentPixels = 0;
const pixelAt = (x, y) => {
  const offset = (y * decodedOnlineIcon.width + x) * 4;
  return decodedOnlineIcon.pixels.subarray(offset, offset + 4);
};
for (let offset = 0; offset < decodedOnlineIcon.pixels.length; offset += 4) {
  const red = decodedOnlineIcon.pixels[offset];
  const green = decodedOnlineIcon.pixels[offset + 1];
  const blue = decodedOnlineIcon.pixels[offset + 2];
  const alpha = decodedOnlineIcon.pixels[offset + 3];
  if (alpha === 0) transparentPixels += 1;
  else if (red === 0 && green === 0 && blue === 0 && alpha === 255) blackPixels += 1;
  else if (red === 255 && green === 255 && blue === 255 && alpha === 255) whitePixels += 1;
}
if (blackPixels < 650 || whitePixels < 300 || transparentPixels < 1_500)
  throw new Error('Online menu icon must retain its black/white contrast keyline contract');
for (const [x, y] of [
  [29, 4], // globe
  [44, 47], // left eye
  [51, 47], // right eye
  [66, 58], // paddle
]) {
  if (!pixelAt(x, y).equals(Buffer.from([0, 0, 0, 255])))
    throw new Error(`Online menu icon is missing its black landmark at ${x},${y}`);
}
const bootstrap = await read('mod/standalone/lovely/bootstrap.toml');
if (!bootstrap.includes('{{lovely_hack:patch_dir}}'))
  throw new Error("Standalone bootstrap must use Lovely's supported patch_dir placeholder");
if (bootstrap.includes('{{lovely_hack::patch_dir}}'))
  throw new Error('Standalone bootstrap contains the invalid double-colon patch_dir placeholder');

for (const contract of [
  [core, 'protocolVersion = 3'],
  [core, 'function BBT.startOnlineRuntime()'],
  [core, 'function BBT.exitOnline()'],
  [core, "gsub('[\\r\\n]', '')"],
  [online, 'BBT.startOnlineRuntime()'],
  [core, "BBT.command('runtime.session_end'"],
  [ipc, 'runtime-path.txt'],
  [ipc, 'CreateProcessA'],
  [ipc, 'beatblock-online-v3'],
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
  [core, 'local renderPlaying = inGame and not cs.startPending and not cs.paused'],
  [core, "function BBT.shouldBlockPause()\n  return BBT.context.lobbyId ~= 'offline'\nend"],
  [hooks, 'if BBT and BBT.shouldBlockPause() then return end'],
  [core, "['render.sample'] = 'bbt_render_latest'"],
  [core, 'outbound:getCount()>=limit'],
  [core, 'processed<MAX_INBOUND_PER_FRAME'],
  [core, 'updatedAtMs = estimatedServerTimeMs()'],
  [renderer, 'readbackPending = {false,false}'],
  [renderer, 'readbackRequests = {nil,nil}'],
  [renderer, 'readbackTickets = {0,0}'],
  [renderer, 'readbackStartedAt = {nil,nil}'],
  [renderer, 'function Renderer.reclaimStalledReadbacks(now)'],
  [renderer, 'dpiscale=1'],
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
  [renderer, 'function Renderer.captureSafe(cleanSource, shadedSource, finalShader)'],
  [renderer, 'function Renderer.capturePlayerView(cleanSource, shadedSource)'],
  [renderer, 'function Renderer.clearPreviousState()'],
  [renderer, "local source = Renderer.mode == 'full' and shadedSource or cleanSource"],
  [renderer, "love.graphics.push('all')"],
  [renderer, 'love.graphics.setShader(finalShader)'],
  [renderer, "love.graphics.setBlendMode('alpha', 'alphamultiply')"],
  [renderer, 'love.graphics.pop()'],
  [
    renderer,
    'local scale = math.min(Renderer.width / sourceWidth, Renderer.height / sourceHeight)',
  ],
  [renderer, 'dataSize ~= Renderer.frameSize'],
  [renderer, 'Renderer.readbackRequests = {nil,nil}'],
  [renderer, 'Renderer.frames.pointer + 32'],
  [renderer, 'function Renderer.steerPaddle()'],
  [renderer, 'function Renderer.applyClock()'],
  [renderer, 'function Renderer.afterGameUpdate()'],
  [renderer, "mouse.circleSnap = 'disabled'"],
  [renderer, 'local circleX, circleY = math.cos(radians) * radius'],
  [renderer, 'math.abs(sourceBeat - Renderer.beat) > .20'],
  [online, 'local initialWorkspace=options.workspace'],
  [online, "BBT.command('room.commentator_set'"],
  [online, "BBT.command('broadcast.mirror_set'"],
  [online, "BBT.command('chart.transfer_decision'"],
  [online, "modal.error='PASSWORD IS REQUIRED'"],
  [online, 'if problem then'],
  [online, 'self.holdEntityDraw=true'],
  [online, 'local function clearNativeEntities(self)'],
  [online, 'clearNativeEntities(self)'],
  [online, 'applyBeatblockMenuFont()'],
  [online, 'st:setBgDraw(function(self)'],
  [components, 'font:getHeight()'],
  [components, 'height < 22'],
  [online, 'local focused=enabled~=false and register'],
  [online, 'local nextByte=value:byte(finalByte+1)'],
  [ipc, 'local pendingSend = nil'],
  [ipc, 'local function rememberHandshake(value)'],
  [ipc, 'local MAX_INBOUND_BACKLOG = 512'],
  [ipc, 'while inbound:getCount()<MAX_INBOUND_BACKLOG'],
  [ipc, 'receiveRemainder=receiveRemainder..partial'],
  [ipc, "not remainder:find('\\n',1,true) and #remainder>MAX_IPC_FRAME"],
]) {
  if (!contract[0].includes(contract[1]))
    throw new Error(`Lazy runtime contract is missing ${contract[1]}`);
}
if (core.includes('function BBT.onPause()') || hooks.includes('BBT.onPause()'))
  throw new Error(
    'Online pause handling must block the native pause instead of invalidating afterward',
  );
if (!core.includes("anchor.sent = BBT.send('render.anchor'"))
  throw new Error('First-note anchors do not retry after bounded IPC backpressure');
if (ipc.includes('"version":2'))
  throw new Error('IPC worker still emits retired protocol-v2 local status envelopes');
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
if (
  !obsPlugin.includes('gs_effect_get_param_by_name(draw, "image")') ||
  !obsPlugin.includes('gs_effect_set_texture_srgb(image, ctx->texture)')
)
  throw new Error('OBS custom-draw source does not bind its frame texture to the base effect');
const playerViewCapture = 'BBTRenderer.capturePlayerView(cs.canv, shuv and shuv.canvasShaded)';
if (!hooks.includes(playerViewCapture))
  throw new Error('Renderer capture hook does not receive raw and final shaded gameplay');
if (!hooks.includes('pattern = "cs:draw()"'))
  throw new Error('Renderer capture does not run after the complete gamestate composition');
if (!hooks.includes("cs.name == 'Game' or cs.name == 'Results'"))
  throw new Error('Renderer capture does not preserve the native Results screen');
if (hooks.includes('BBTRenderer.beginGameplayOnly(self)'))
  throw new Error('Renderer still suppresses chart scenery and background effects');
if (hooks.includes('BBTRenderer.captureSafe(self.canv, shuv.canvas)'))
  throw new Error('Renderer still exposes Beatblock palette-index colors as the Full stream');
if (!renderer.includes('chromatic and chromatic.enabled'))
  throw new Error('Renderer Full mode omits Beatblock final chromatic-aberration pass');
if (renderer.includes('cs and cs.notes') || renderer.includes('local function drawClean()'))
  throw new Error('Renderer still publishes the synthetic empty clean-mode scene');
if (!hooks.includes('BBTRenderer.shouldHold() and not self.startPending then return end'))
  throw new Error('Renderer hold can block Beatblock before chart preloading completes');
if (
  !renderer.includes('function Renderer.shouldFreezeSimulation()') ||
  !hooks.includes('BBTRenderer.shouldFreezeSimulation()') ||
  !hooks.includes('pattern = "prof.push(\\\"flux update\\\")"') ||
  !hooks.includes('pattern = "prof.pop(\\\"entityman update\\\")"')
)
  throw new Error('Renderer hold does not freeze Beatblock flux and EntityManager together');
if (!renderer.includes('Renderer.clearPreviousState()\n  cs.bbtRenderer = true'))
  throw new Error('Renderer enters Game without releasing menu entities and eases');
const nativeClockBoundary = hooks.indexOf('target = "obj/GameManager.lua"');
const remoteClock = hooks.indexOf('BBTRenderer.applyClock()');
if (nativeClockBoundary < 0 || remoteClock < nativeClockBoundary)
  throw new Error(
    "Renderer does not apply the delayed beat at GameManager's native event boundary",
  );
const broadcastBody = online.slice(
  online.indexOf('local function drawBroadcast(self)'),
  online.indexOf('local function drawHistory(self)'),
);
if (
  !broadcastBody.includes('local room=currentRoom()') ||
  !broadcastBody.includes("local rendererEditable=room and room.lifecycle~='playing'")
)
  throw new Error('Broadcast workspace reads lifecycle state without a local room snapshot');
const captureBody = renderer.slice(
  renderer.indexOf('function Renderer.capture(cleanSource, shadedSource, finalShader)'),
  renderer.indexOf('function Renderer.captureSafe(cleanSource, shadedSource, finalShader)'),
);
if (captureBody.includes('Renderer.update()'))
  throw new Error('Renderer capture consumes input after gameplay simulation has already run');
if (!online.includes("mode='full'"))
  throw new Error('Broadcast assignment does not default to the chart-faithful full composition');
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
if (
  !hooks.includes('loc.json.bbtOnline') ||
  !hooks.includes("BBT.assetImage('assets/online.png')") ||
  !hooks.includes('sprites.menu.bbtOnline')
) {
  throw new Error('Online menu entry must provide a localized label and its branded icon');
}
if (!core.includes('function BBT.assetImage(relativePath)'))
  throw new Error('Online menu icon is missing its external asset loader');
if (
  core.includes("BBT.send('client.hello'") &&
  core.indexOf("BBT.send('client.hello'") < core.indexOf('function BBT.startOnlineRuntime()')
)
  throw new Error('Runtime hello is sent before entering Online');
if (`${core}\n${online}`.includes('manager.open_request'))
  throw new Error('Obsolete visible Manager command remains');

for (const contrastContract of [
  'muted={1,0,0,1}',
  'applyBeatblockPalette(false)',
  'applyBeatblockPalette(true)',
  'function ui:veil()',
  'or self.palette.black',
  "enabled and 'black' or 'muted'",
  "selected and 'black'",
]) {
  if (!`${online}\n${components}`.includes(contrastContract))
    throw new Error(`Online font contrast contract is missing ${contrastContract}`);
}
if (!online.includes('shuv.showBadColors=showBadColors'))
  throw new Error('Online palette lifecycle does not explicitly restore the native menu shader');
if (/muted=\{(?:\.\d+|1,1,1,\.)/.test(online))
  throw new Error('Online muted copy still uses a palette-unstable RGB or alpha color');
if (online.includes("ui:color('black',.78)"))
  throw new Error('Online modal still uses palette-unstable alpha dimming');
if (online.includes("'connect_host'"))
  throw new Error('Connect duplicates the session strip Host action');
if (online.includes('local PAGES ='))
  throw new Error('Online still uses the obsolete six-page tab bar');
for (const dashboardContract of [
  "require('bbt.dashboard_model')",
  "{id='setlist',label='SETLIST'}",
  "{id='broadcast',label='BROADCAST'}",
  "self.rosterFilter='all'",
  'self.selectedSessionId',
  'local function openConfirm',
  'local function drawHelp',
  'local function drawBroadcast',
  "self.workspace~='room'",
]) {
  if (!online.includes(dashboardContract))
    throw new Error(`Concentrated dashboard contract is missing ${dashboardContract}`);
}
for (const modelContract of [
  'function Dashboard.phase(context)',
  'function Dashboard.summary(context)',
  'function Dashboard.primary(context)',
  'function Dashboard.visibleParticipants(context, filter)',
  'function Dashboard.selectedParticipant(context, filter, sessionId)',
  'function Dashboard.score(participant, lifecycle)',
  'function Dashboard.canBroadcast(context)',
  "return action('select_chart','SELECT CHART'",
  "return action('locate_chart','FIND MATCHING CHART'",
  "return action('start_race','START RACE'",
  "return action('advance_set','NEXT CHART'",
  "return action('select_next_chart','SELECT NEXT CHART'",
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
  'room.host_play_set',
  'room.validity_checks_set',
  'room.commentator_set',
  'room.kick',
  'setlist.remove',
  'setlist.move',
  'setlist.advance',
  'renderer.configure',
  'broadcast.mirror_set',
  'chart.transfer_request',
  'chart.transfer_decision',
  'chart.cache_clear',
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
  'JOIN AS PLAYER',
  'JOIN AS SPECTATOR',
  'CHOOSE HOW YOU JOIN',
  'SELECT OFFICIAL',
  'SELECT CUSTOM',
  'FIND MATCHING CHART',
  'READY',
  'START RACE',
  'PLAY NEXT RACE',
  'CONTINUE TO NEXT CHART',
  'ADVANCED OBS EXPORT',
  '1920 x 1080',
  'BROADCAST',
  'MATCH HISTORY',
  'TRANSFER CACHE',
  'COMMENTATOR',
]) {
  if (!`${online}\n${dashboard}`.includes(capability))
    throw new Error(`Online dashboard is missing ${capability}`);
}
if (!core.includes('if sent then BBT.runSequence = BBT.runSequence + 1 end'))
  throw new Error('Rejected score IPC writes still create false run-sequence gaps');
if (
  !core.includes('local MAX_STANDARD_OUTBOUND = 480') ||
  !core.includes("['run.finished'] = true")
)
  throw new Error('Run lifecycle messages do not retain reserved ordered IPC capacity');
if (
  !core.includes('BBT.scoreDirty=true') ||
  !core.includes('flushScoreDelta(true)') ||
  !core.includes('BBT.wasRunReady=false')
)
  throw new Error('Native score mutations and retry boundaries are not flushed safely');
if (!hooks.includes('BBT.shouldBlockRetry()'))
  throw new Error('Results retry is not governed by the host Run Checks policy');
for (const interaction of [
  "pressed('select')",
  'mouse.pressed==1',
  'love.keyboard.setTextInput',
  "key~='backspace' and key~='delete'",
  'pcall(utf8.offset,value,-1)',
  'love.keypressed=self.onlineKeyPressed',
  'love.keypressed=self.previousKeyPressed',
  'HOST ADDRESS',
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
for (const handoff of ['self.menuMusicManager:update(dt)', 'cs.menuMusicManager=music']) {
  if (!online.includes(handoff)) throw new Error(`Online music continuity is missing ${handoff}`);
}
for (const handoff of [
  'local function returnFromChartSelector(selector, returnWorkspace)',
  'music:forceUnmute()',
  'cs.menuMusicManager = music',
  "cs:init({workspace=returnWorkspace or 'room'})",
  "return (mode == 'host' or mode == 'setlist') and 'setlist' or 'room'",
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
    'dashboard_components.lua',
    'online_state.lua',
    'ipc_thread.lua',
    'renderer.lua',
  ]) {
    const shared = await read(`mod/shared/bbt/${file}`);
    const packaged = await read(`mod/${distribution}/bbt/${file}`);
    if (hash(shared) !== hash(packaged))
      throw new Error(`${distribution}/${file} was not generated from the shared core`);
  }
  const packagedIcon = await readFile(resolve(root, `mod/${distribution}/assets/online.png`));
  if (hash(onlineIcon) !== hash(packagedIcon))
    throw new Error(`${distribution}/assets/online.png was not generated from the shared asset`);
  const archive = resolve(root, `mod/releases/beatblock-online-${distribution}-0.3.0-beta.1.zip`);
  const entries = new Set(
    listZipEntries(await readFile(archive), `${distribution} release ZIP`).map((entry) =>
      entry.replaceAll('\\', '/'),
    ),
  );
  const prefix = 'BeatblockOnline/';
  const distributionFiles =
    distribution === 'standalone'
      ? ['lovely/bootstrap.toml', 'README.txt']
      : ['mod.json', 'main.lua', 'config.lua', 'states/Online.lua', 'README.txt'];
  for (const required of [
    'assets/online.png',
    'bbt/core.lua',
    'bbt/dashboard_model.lua',
    'bbt/dashboard_components.lua',
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
