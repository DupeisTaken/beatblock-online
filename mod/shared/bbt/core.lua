local BBT = {
  version = '0.3.0-beta.5',
  protocolVersion = 3,
  testedBeatblockVersion = '1.7.1a',
  sequence = 0,
  runSequence = 0,
  snapshotTimer = 0,
  renderTimer = 0,
  keyframeTimer = 0,
  installedHooks = false,
  connected = false,
  companionConnected = false,
  disabled = false,
  context = { lobbyId = 'offline', runId = nil, playerName = 'Player', lobbyName = 'Offline practice' },
  scheduledStartTimeMs = nil,
  localChart = nil,
  chartVerified = false,
  selectingOnlineChart = false,
  chartSelectionMode = nil,
  chartSelectionPrevious = nil,
  lastError = nil,
  launching = false,
  clockSynchronized = false,
  wasInGame = false,
  wasRunReady = false,
  sessionActive = false,
  requestSequence = 0,
  hudEnabled = true,
  scoreDirty = false,
  resultsReportedRunId = nil,
}

local CLIENT_INSTANCE_ID = tostring(os.time())..'-'..tostring(math.random(100000,999999))
local DEFAULT_COMMAND_TIMEOUT_MS = 10000
local COMMAND_TIMEOUT_MS = {
  ['room.host_request'] = 12000,
  ['room.join_request'] = 20000,
  ['runtime.restart_request'] = 15000,
}
-- Ordered control/score traffic must remain bounded while the runtime is down.
-- At 512 small JSON frames the game retains a useful recovery window without
-- allowing a long offline session to consume unbounded process memory.
local MAX_ORDERED_OUTBOUND = 512
local MAX_STANDARD_OUTBOUND = 480
local MAX_INBOUND_PER_FRAME = 128
local RUN_LIFECYCLE_MESSAGES = {
  ['run.started'] = true,
  ['run.invalid'] = true,
  ['run.finished'] = true,
}
local COALESCED_CHANNELS = {
  ['render.sample'] = 'bbt_render_latest',
  ['render.keyframe'] = 'bbt_keyframe_latest',
  ['gameplay.snapshot'] = 'bbt_snapshot_latest',
  ['client.ping'] = 'bbt_heartbeat_latest',
}

local function monotonicMs()
  return love and love.timer and math.floor(love.timer.getTime() * 1000) or 0
end

-- os.time has one-second resolution. Anchor it once to LÖVE's monotonic clock
-- so pre-handshake timestamps still advance in milliseconds.
local WALL_CLOCK_OFFSET_MS = os.time() * 1000 - monotonicMs()
local function nowMs()
  return WALL_CLOCK_OFFSET_MS + monotonicMs()
end

local function estimatedServerTimeMs()
  if BBT.clockSynchronized then return monotonicMs() + BBT.serverMonotonicOffsetMs end
  return nowMs()
end

local function encode(value)
  -- Beatblock's bundled rxi encoder appends a raw newline after every object,
  -- including nested payloads. Strings are escaped first, so stripping raw
  -- CR/LF here safely restores the protocol's one-envelope-per-line framing.
  if json and json.encode then return (json.encode(value):gsub('[\r\n]', '')) end
  if dpf and dpf.json and dpf.json.encode then return (dpf.json.encode(value):gsub('[\r\n]', '')) end
  error('Beatblock Online could not find the game JSON encoder')
end

local function decode(value)
  if json and json.decode then return json.decode(value) end
  if dpf and dpf.json and dpf.json.decode then return dpf.json.decode(value) end
  return nil
end

local function accuracy(totals)
  if totals.currentMaxHits <= 0 then return 100 end
  return math.max(0, math.floor((((totals.currentMaxHits - totals.misses - totals.barelies / 4) / totals.currentMaxHits) * 100) * 100) / 100)
end

local function resultsAccuracy(totals)
  if totals.maxHits <= 0 then return 100 end
  -- Results uses the chart-wide maximum rather than the live HUD denominator.
  -- Keep the source keyframe identical to the player's native Results screen.
  return math.floor((((totals.maxHits - totals.misses - totals.barelies / 4) / totals.maxHits) * 100) * 100) / 100
end

local function totals()
  local mineHits = 0
  if cs and type(cs.mineHits) == 'table' then for _ in pairs(cs.mineHits) do mineHits = mineHits + 1 end end
  return {
    hits = math.max(0, cs and cs.hits or 0),
    misses = math.max(0, cs and cs.misses or 0),
    barelies = math.max(0, cs and cs.barelies or 0),
    combo = math.max(0, cs and cs.combo or 0),
    maxCombo = math.max(0, cs and cs.maxCombo or 0),
    currentMaxHits = math.max(0, cs and cs.currentMaxHits or 0),
    maxHits = math.max(0, cs and cs.maxHits or 0),
    mineHits = mineHits,
  }
end

local function averageTapOffset()
  if not (cs and type(cs.tapTiming) == 'table') or #cs.tapTiming == 0 then return 0 end
  local sum = 0
  for _, value in ipairs(cs.tapTiming) do sum = sum + (tonumber(value) or 0) end
  return math.floor((sum / #cs.tapTiming) * 1000) / 1000
end

function BBT.send(kind, payload, critical)
  if BBT.disabled or not BBT.sessionActive then return end
  local message = {
    version = BBT.protocolVersion,
    type = kind,
    sequence = BBT.sequence,
    runTimeUs = math.floor((love and love.timer and love.timer.getTime() or 0) * 1000000),
    runId = BBT.context.runId,
    requestId = payload and payload.requestId or nil,
    payload = payload,
  }
  BBT.sequence = BBT.sequence + 1
  local encoded=encode(message)
  local latestName=COALESCED_CHANNELS[kind]
  if latestName then
    -- Presentation/liveness telemetry is latest-value state. Replacing it keeps
    -- a disconnected runtime from accumulating minutes of obsolete samples.
    local latest=love.thread.getChannel(latestName)
    latest:clear()
    latest:push(encoded)
    return true
  end
  local outbound=love.thread.getChannel('bbt_outbound')
  local limit=(critical or RUN_LIFECYCLE_MESSAGES[kind]) and MAX_ORDERED_OUTBOUND
    or MAX_STANDARD_OUTBOUND
  if outbound:getCount()>=limit then
    BBT.lastError='Online IPC is overloaded; reconnect before continuing the race.'
    return false
  end
  outbound:push(encoded)
  return true
end

function BBT.command(kind, payload)
  if BBT.pendingRequestId and kind ~= 'runtime.session_end' then
    BBT.lastError = 'Please wait for the current Online action to finish.'
    return nil
  end
  BBT.lastError = nil
  payload = payload or {}
  BBT.requestSequence = BBT.requestSequence + 1
  local requestId = CLIENT_INSTANCE_ID .. '-' .. tostring(BBT.requestSequence)
  payload.requestId = requestId
  BBT.pendingRequestId = requestId
  BBT.pendingRequestKind = kind
  BBT.pendingRequestProgress = nil
  BBT.pendingRequestDeadlineMs = monotonicMs() + (COMMAND_TIMEOUT_MS[kind] or DEFAULT_COMMAND_TIMEOUT_MS)
  if not BBT.send(kind, payload) then
    BBT.pendingRequestId = nil
    BBT.pendingRequestKind = nil
    BBT.pendingRequestProgress = nil
    BBT.pendingRequestDeadlineMs = nil
    return nil
  end
  return requestId
end

-- Every command has exactly one terminal state. Centralizing cleanup prevents
-- a lost transport, timeout, or late reply from leaving Online permanently busy.
local function clearPendingRequest()
  BBT.pendingRequestId = nil
  BBT.pendingRequestKind = nil
  BBT.pendingRequestDeadlineMs = nil
  BBT.pendingRequestProgress = nil
end

-- The native engine is deliberately lazy: normal Beatblock menus never start it.
function BBT.startOnlineRuntime()
  if BBT.sessionActive or (BBTRenderer and BBTRenderer.active) then return end
  BBT.sessionActive = true
  BBT.companionConnected = false
  BBT.runtimeStarting = true
  BBT.scoreDirty = false
  clearPendingRequest()
  love.thread.getChannel('bbt_ipc_control'):clear()
  love.thread.getChannel('bbt_outbound'):clear()
  love.thread.getChannel('bbt_inbound'):clear()
  for _,channelName in pairs(COALESCED_CHANNELS) do
    love.thread.getChannel(channelName):clear()
  end
  local threadPath = BBT.modPath .. '/bbt/ipc_thread.lua'
  -- Lovely's patch directory lives outside LÖVE's virtual filesystem. Read the
  -- worker once while entering Online, then hand LÖVE a FileData object so the
  -- gameplay loop never performs IPC or filesystem work.
  local sourceFile = io.open(threadPath, 'rb')
  local source = sourceFile and sourceFile:read('*a') or nil
  if sourceFile then sourceFile:close() end
  local ok, thread = pcall(function()
    if not source then error('IPC worker is missing from the installed mod') end
    return love.thread.newThread(source)
  end)
  if ok then
    BBT.ipcThread = thread
    local pathChannel = love.thread.getChannel('bbt_mod_path')
    pathChannel:clear(); pathChannel:push(BBT.modPath)
    thread:start()
    -- The Lua process is not a trust boundary. `version` is Beatblock's own
    -- menu label, including the bracketed upstream build token shown in the
    -- top-right corner. The runtime validates and normalizes that value before
    -- any room handshake, with a streamed game-content digest as its fallback.
    BBT.send('client.hello', {
      instanceId=CLIENT_INSTANCE_ID,
      clientVersion=BBT.version,
      gameVersion=type(version)=='string' and version or '',
      distribution=BBT.distribution,
      mods={},
    })
  else
    BBT.sessionActive = false
    BBT.lastError = 'Could not start the Beatblock Online runtime IPC: ' .. tostring(thread)
  end
end

function BBT.exitOnline()
  if not BBT.sessionActive then return end
  -- Shutdown control must not sit behind stale 60 Hz render samples. Ordered
  -- run journals are already persisted by the runtime; unsent UI telemetry is
  -- expendable once the user explicitly leaves Online.
  love.thread.getChannel('bbt_outbound'):clear()
  for _,channelName in pairs(COALESCED_CHANNELS) do
    love.thread.getChannel(channelName):clear()
  end
  BBT.command('runtime.session_end', {})
  clearPendingRequest()
  BBT.sessionActive = false
  BBT.connected = false
  BBT.companionConnected = false
  BBT.runtimeStarting = false
  BBT.scoreDirty = false
  BBT.lastLobby = nil
  BBT.context.lobbyId = 'offline'
  love.thread.getChannel('bbt_ipc_control'):push('stop')
end

function BBT.openInstaller()
  local file = io.open(BBT.modPath .. '/installer-path.txt', 'rb')
  local path = file and file:read('*a') or nil
  if file then file:close() end
  if not path or path == '' then BBT.lastError = 'Installer maintenance copy is missing. Download BeatblockOnlineInstaller.exe again.'; return end
  path = path:gsub('[\r\n]+$', '')
  local ok, ffi = pcall(require, 'ffi')
  if not ok then BBT.lastError = 'Windows launcher is unavailable.'; return end
  -- `ffi.C` only resolves symbols from libraries this process already imported,
  -- and an unresolved symbol raises rather than returning nil. Load shell32
  -- explicitly and keep the declaration and dispatch inside one pcall so the
  -- repair screen reports a message instead of throwing out of the callback.
  local launched, result = pcall(function()
    ffi.cdef[[void* ShellExecuteA(void*, const char*, const char*, const char*, const char*, int);]]
    local shell32 = ffi.load('shell32')
    return tonumber(ffi.cast('intptr_t', shell32.ShellExecuteA(nil, 'open', path, nil, nil, 1)))
  end)
  if not launched or not result or result <= 32 then BBT.lastError = 'Windows could not open the installer maintenance copy.' end
end

-- Lovely's patch directory is outside LÖVE's virtual filesystem. Read visual
-- assets through normal I/O, then wrap the bytes in FileData for the renderer.
-- The cache prevents repeated menu visits from allocating duplicate textures.
function BBT.assetImage(relativePath)
  BBT.assetImages = BBT.assetImages or {}
  if BBT.assetImages[relativePath] ~= nil then
    return BBT.assetImages[relativePath] or nil
  end
  local file = io.open(BBT.modPath .. '/' .. relativePath, 'rb')
  local bytes = file and file:read('*a') or nil
  if file then file:close() end
  if not bytes then
    BBT.assetImages[relativePath] = false
    if log then log('Beatblock Online asset is missing: '..relativePath, 'warning') end
    return nil
  end
  local ok, image = pcall(function()
    local data = love.filesystem.newFileData(bytes, relativePath)
    return love.graphics.newImage(data)
  end)
  BBT.assetImages[relativePath] = ok and image or false
  if not ok and log then log('Beatblock Online could not load asset '..relativePath..': '..tostring(image), 'warning') end
  return ok and image or nil
end

function BBT.currentPlayer()
  if not BBT.lastLobby or not BBT.lastLobby.participants then return nil end
  for _, player in ipairs(BBT.lastLobby.participants) do
    if player.sessionId == BBT.context.sessionId or player.displayName == BBT.context.playerName then
      return player
    end
  end
  return nil
end

function BBT.isOrganizer()
  return BBT.lastLobby and BBT.lastLobby.hostSessionId and BBT.currentPlayer()
    and BBT.lastLobby.hostSessionId == BBT.currentPlayer().sessionId
end

function BBT.openChartSelect(mode)
  if BBT.pendingRequestId then
    BBT.lastError = 'Wait for the current Online action before selecting a chart.'
    return
  end
  BBT.selectingOnlineChart = true
  BBT.chartSelectionMode = mode or 'verify'
  BBT.chartSelectionPrevious = BBT.chartSelectionMode == 'setlist' and {
    chart = BBT.localChart,
    verified = BBT.chartVerified,
  } or nil
  local previous = cs
  local menuMusicManager = previous and previous.menuMusicManager
  cs = bs.load('SongSelect')
  if previous and previous.leave then previous:leave() end
  if menuMusicManager then menuMusicManager:clearOnBeatHooks() end
  cs.menuMusicManager = menuMusicManager
  cs.topDirectory = 'Custom Levels/'
  cs.allowEditor = false
  cs.bbtOnlineSelection = true
  cs:init()
end

local expectedMaxHits

local function chartPreloadReady(levelData, soundData)
  if not levelData or not levelData.events then return false end
  for _, event in ipairs(levelData.events) do
    if event.type == 'play' then return soundData ~= nil and soundData ~= false end
  end
  return true
end

function BBT.openOfficialSelect(mode)
  if BBT.pendingRequestId then
    BBT.lastError = 'Wait for the current Online action before selecting a chart.'
    return
  end
  -- Freeplay's SongSelect owns one coherent chart/audio preload pair. Atom
  -- Map's asynchronous quark state cannot be transferred safely into an
  -- independently scheduled online launch.
  BBT.selectingOnlineChart = true
  BBT.selectingOfficialChart = true
  BBT.chartSelectionMode = mode or 'verify'
  BBT.chartSelectionPrevious = BBT.chartSelectionMode == 'setlist' and {
    chart = BBT.localChart,
    verified = BBT.chartVerified,
  } or nil
  local previous = cs
  local music = previous and previous.menuMusicManager
  cs = bs.load('SongSelect')
  if previous and previous.leave then previous:leave() end
  if music then music:clearOnBeatHooks() end
  cs.menuMusicManager = music
  cs.topDirectory = 'levels/Songwheel/'
  cs.allowEditor = false
  cs.bbtOnlineSelection = true
  cs:init()
end

-- SongSelect normally returns to the main menu. Online selection owns the
-- state, so success and cancellation return to the originating workspace.
local function chartSelectionReturnWorkspace(mode)
  return (mode == 'host' or mode == 'setlist') and 'setlist' or 'room'
end

local function returnFromChartSelector(selector, returnWorkspace)
  local music = selector and selector.menuMusicManager
  if selector and selector.source then selector.source:stop(); selector.source = nil end
  if selector and selector.resetLevelPreload then pcall(selector.resetLevelPreload, selector) end
  if selector and selector.deletePlayer then pcall(selector.deletePlayer, selector) end
  if music then music:clearOnBeatHooks(); music:forceUnmute() end
  cs = bs.load('Online')
  cs.menuMusicManager = music
  -- Re-enter the workspace that launched Song Select. Adding another setlist
  -- chart should be a continuous workflow, not an implicit navigation reset.
  cs:init({workspace=returnWorkspace or 'room'})
end

function BBT.cancelChartSelection(selector)
  if not BBT.selectingOnlineChart and not BBT.selectingOfficialChart then return false end
  local returnWorkspace = chartSelectionReturnWorkspace(BBT.chartSelectionMode)
  BBT.selectingOnlineChart = false
  BBT.selectingOfficialChart = false
  BBT.chartSelectionMode = nil
  BBT.chartSelectionPrevious = nil
  returnFromChartSelector(selector, returnWorkspace)
  return true
end

local function selectedPackagePath(levelPath)
  -- SongSelect accepts both manifest-based charts and legacy level.json
  -- packages. Resolve the same file that made the row selectable, and keep the
  -- UTF-8 virtual path untouched when joining it to the physical mount root.
  local real
  for _, metadata in ipairs({'manifest.json', 'level.json'}) do
    real = love.filesystem.getRealDirectory(levelPath .. metadata)
    if real then break end
  end
  if not real then return nil end
  if string.lower(string.sub(real, -4)) == '.zip' then return real end
  local finalCharacter = string.sub(real, -1)
  local separator = (finalCharacter == '/' or finalCharacter == '\\') and '' or '/'
  return real .. separator .. levelPath
end

expectedMaxHits = function(levelData)
  if not levelData or not levelData.events or not Event or not Event.hitCount then return nil end
  local total, mineBeats = 0, {}
  for _, event in ipairs(levelData.events) do
    if event.type == 'mine' then
      if not mineBeats[event.time] then mineBeats[event.time] = true; total = total + 1 end
    elseif event.type == 'mineHold' then
      local beat = (event.time or 0) + (event.duration or 0)
      if not mineBeats[beat] then mineBeats[beat] = true; total = total + 1 end
    elseif Event.hitCount[event.type] then
      local ok, count = pcall(Event.hitCount[event.type], event)
      if ok and type(count) == 'number' then total = total + count end
    end
  end
  return math.floor(total)
end

function BBT.onChartSelected(selector, levelPath, variantName)
  local item = selector.menuItems and selector.menuItems[selector.selection]
  if not item then
    BBT.cancelChartSelection(selector)
    BBT.lastError = 'Beatblock did not expose the selected chart'
    return
  end
  local officialSelection = BBT.selectingOfficialChart
  local selectionMode = BBT.chartSelectionMode
  local previousSelection = BBT.chartSelectionPrevious
  -- The selected row is authoritative. Using its filename avoids deriving a
  -- package path from the rendered/localized song title, which may contain
  -- punctuation or multi-byte UTF-8 characters.
  levelPath = type(item.filename) == 'string' and item.filename or levelPath
  local variantInfo = selector.getVariantInfo and selector:getVariantInfo(item, variantName) or nil
  local packagePath = not officialSelection and selectedPackagePath(levelPath) or nil
  BBT.localChart = {
    levelPath = levelPath,
    packagePath = packagePath,
    variantName = variantName or 'Default',
    variantInfo = variantInfo,
    levelData = selector.levelData,
    soundData = selector.preloadSoundData,
    songName = (item.rawMetadata and item.rawMetadata.songName) or item.name or 'Unknown chart',
    expectedMaxHits = expectedMaxHits(selector.levelData),
    official = officialSelection,
  }
  BBT.chartVerified = false
  BBT.selectingOnlineChart = false
  BBT.selectingOfficialChart = false
  local selectionError
  local preloadReady = chartPreloadReady(BBT.localChart.levelData, BBT.localChart.soundData)
  if officialSelection and preloadReady and BBT.localChart.expectedMaxHits and BBT.localChart.expectedMaxHits > 0 then
    BBT.command((selectionMode == 'host' or selectionMode == 'single' or selectionMode == 'setlist') and 'room.official_chart_select' or 'room.official_chart_verify', {
      chartId = levelPath,
      songName = BBT.localChart.songName,
      variant = BBT.localChart.variantName,
      expectedMaxHits = BBT.localChart.expectedMaxHits,
      appendToSetlist = selectionMode == 'setlist',
    })
  elseif packagePath and preloadReady and BBT.localChart.expectedMaxHits and BBT.localChart.expectedMaxHits > 0 then
    BBT.command((selectionMode == 'host' or selectionMode == 'single' or selectionMode == 'setlist') and 'room.chart_select_request' or 'room.chart_verify_request', {
      path = packagePath,
      levelPath = levelPath,
      songName = BBT.localChart.songName,
      variant = BBT.localChart.variantName,
      expectedMaxHits = BBT.localChart.expectedMaxHits,
      appendToSetlist = selectionMode == 'setlist',
    })
  elseif (officialSelection or packagePath) and not preloadReady then
    selectionError = 'Chart audio is still loading; wait for the Freeplay preview and select it again'
  elseif officialSelection or packagePath then
    selectionError = 'Chart notes were not preloaded; wait for the Freeplay preview and select it again'
  else
    selectionError = 'Could not resolve the selected level package on disk'
  end
  if selectionMode == 'setlist' and previousSelection and previousSelection.chart then
    BBT.localChart = previousSelection.chart
    BBT.chartVerified = previousSelection.verified
  end
  BBT.chartSelectionMode = nil
  BBT.chartSelectionPrevious = nil
  returnFromChartSelector(selector, chartSelectionReturnWorkspace(selectionMode))
  if selectionError then BBT.lastError = selectionError end
end

function BBT.maybeLaunchScheduledChart()
  if BBT.launching or not BBT.lastLobby or BBT.lastLobby.lifecycle ~= 'countdown' then return false end
  if not BBT.localChart or not BBT.chartVerified or not BBT.scheduledStartTimeMs then return false end
  if estimatedServerTimeMs() < BBT.scheduledStartTimeMs - 3500 then return false end
  BBT.launching = true
  cLevel = BBT.localChart.levelPath
  returnData = { state = 'Online', vars = {} }
  local previous = cs
  cs = bs.load('Game')
  -- Online owns the menu manager and may retain a selector preview source.
  -- Stop both before Game init so intro/menu audio cannot leak into the chart.
  if previous and previous.source then
    pcall(previous.source.stop, previous.source)
    previous.source = nil
  end
  if previous and previous.menuMusicManager then
    previous.menuMusicManager:clearOnBeatHooks()
    previous.menuMusicManager:stop()
  end
  if GameManager and GameManager.transferStateData and previous then
    GameManager:transferStateData(cs, previous)
  end
  if previous and previous.leave then previous:leave() end
  cs:init(BBT.localChart.levelPath, BBT.localChart.variantInfo, BBT.localChart.levelData, BBT.localChart.soundData)
  return true
end

function BBT.init(distribution, modPath)
  if _G.BBT_ACTIVE_DISTRIBUTION and _G.BBT_ACTIVE_DISTRIBUTION ~= distribution then
    BBT.disabled = true
    if log then log('Both Beatblock Online packages are installed. Remove one package before playing.', 'warning') end
    return BBT
  end
  _G.BBT_ACTIVE_DISTRIBUTION = distribution
  BBT.distribution = distribution
  BBT.modPath = modPath
  if os.getenv('BBT_RENDERER_FRAME_PATH') then
    local rendererOk, renderer = pcall(require, 'bbt.renderer')
    if rendererOk then renderer.init() end
  end
  BBT.context.runId = 'run_' .. tostring(os.time()) .. '_' .. tostring(math.random(100000, 999999))
  if bs and bs.states and not bs.states.Online then
    local ok, factory = pcall(require, 'bbt.online_state')
    if ok then bs.states.Online = factory end
  end
  if BBTRenderer and BBTRenderer.active then return BBT end
  return BBT
end

-- Score keyframes are source-authored renderer state. The runtime aligns them
-- to the same delayed motion sample used by each OBS slot, so hidden replays do
-- not substitute their own accuracy or Results totals.
local function emitRenderKeyframe(current, results, forceScore, scoreMax)
  current = current or totals()
  if forceScore then
    local forcedMax = tonumber(scoreMax) or 100
    current.maxHits = forcedMax
    current.currentMaxHits = forcedMax
    current.misses = math.max(0, forcedMax - tonumber(forceScore))
    current.barelies = 0
  elseif results and current.maxHits == 0 then
    -- Match Game:goToResults' zero-note guard before the hidden child enters
    -- Results from this source-authored state.
    current.maxHits = 1
  end
  BBT.send('render.keyframe', {
    beat = cs and cs.cBeat or 0,
    paddleAngle = cs and cs.p and cs.p.angle or 0,
    totals = current,
    accuracy = results and resultsAccuracy(current) or accuracy(current),
    averageOffset = averageTapOffset(),
    activeNotes = cs and cs.notes and #cs.notes or 0,
    results = results == true,
  })
end

local function emitScoreDelta(critical, current)
  current = current or totals()
  local songTimeMs = 0
  if cs and cs.source and cs.source.tell then
    local ok, value = pcall(cs.source.tell, cs.source)
    if ok and type(value) == 'number' then songTimeMs = math.floor(value * 1000) end
  end
  local sent=BBT.send('run.score_delta', {
    lobbyId = BBT.context.lobbyId,
    runId = BBT.context.runId,
    runSequence = BBT.runSequence,
    progress = current.maxHits > 0 and math.min(1, current.currentMaxHits / current.maxHits) or 0,
    beat = cs and cs.cBeat or 0,
    songTimeMs = songTimeMs,
    totals = current,
  },critical)
  -- Score totals are cumulative, so a locally rejected queue write can be
  -- recovered by the next update. Only number messages that actually entered
  -- the ordered IPC queue; otherwise one overload creates a false sequence-gap
  -- INVALID result even though no transmitted event was lost in transit.
  if sent then BBT.runSequence = BBT.runSequence + 1 end
  -- OBS is presentation state, so publish its latest source-authored score even
  -- if the ordered scoring queue is temporarily full. The coalesced keyframe
  -- cannot create a validation sequence gap and will be replaced on retry.
  emitRenderKeyframe(current, false)
  return sent
end

local function flushScoreDelta(critical)
  if not BBT.scoreDirty then return true end
  if not emitScoreDelta(critical) then return false end
  BBT.scoreDirty=false
  return true
end

function BBT.installHooks()
  if BBT.installedHooks or not GameManager then return end
  local addToScore = GameManager.addToScore
  local handleMiss = GameManager.handleMiss
  local addMineToTotal = GameManager.addMineToTotal
  local getTapInputs = GameManager.getTapInputs
  local updateTaps = GameManager.updateTaps
  if not addToScore or not handleMiss or not addMineToTotal then return end
  GameManager.addToScore = function(self, ...)
    local before = totals()
    local result = { addToScore(self, ...) }
    local after = totals()
    if after.hits ~= before.hits or after.barelies ~= before.barelies or after.currentMaxHits ~= before.currentMaxHits then BBT.scoreDirty=true end
    return unpack(result)
  end
  GameManager.handleMiss = function(self, ...)
    local before = totals()
    local result = { handleMiss(self, ...) }
    if totals().misses ~= before.misses then BBT.scoreDirty=true end
    return unpack(result)
  end
  GameManager.addMineToTotal = function(self, ...)
    local before = totals()
    local result = { addMineToTotal(self, ...) }
    if totals().currentMaxHits ~= before.currentMaxHits then BBT.scoreDirty=true end
    return unpack(result)
  end
  if getTapInputs then
    GameManager.getTapInputs = function(self, ...)
      if BBTRenderer and BBTRenderer.active then return BBTRenderer.tapInputs() end
      local pressed, released = getTapInputs(self, ...)
      if pressed or released then
        local beat = cs and cs.cBeat or 0
        local offset = savedata and savedata.options and savedata.options.game
          and tonumber(savedata.options.game.inputOffset) or 0
        local judgementBeat = beat
        -- LadybugManager calls this native method with dot syntax and no
        -- receiver. Preserve that valid call shape while using the active game
        -- manager for the optional offset conversion.
        local manager = self or (cs and cs.gm)
        if manager and manager.msToBeat then
          judgementBeat = beat - manager:msToBeat(offset)
        end
        -- Keep the exact native edge and its already offset-adjusted judgement
        -- position. The renderer must not reinterpret it using the host save.
        BBT.send('input.tap', {
          pressed = pressed == true, released = released == true,
          beat = beat, judgementBeat = judgementBeat,
        })
      end
      return pressed, released
    end
  end
  if updateTaps then
    GameManager.updateTaps = function(self, ...)
      if not (BBTRenderer and BBTRenderer.active) then return updateTaps(self, ...) end
      local args = {...}
      local event = BBTRenderer.beginTapJudgement()
      local gameOptions = savedata and savedata.options and savedata.options.game
      local originalOffset = gameOptions and gameOptions.inputOffset
      local originalBeat = cs and cs.cBeat
      -- Reliable edges carry a source-side judgement beat. A raw 60 Hz edge
      -- fallback instead uses the source player's input offset while the native
      -- tap code runs. Either path preserves Beatblock's own scoring effects.
      if gameOptions then
        gameOptions.inputOffset = event and event.judgementBeat and 0
          or BBTRenderer.inputOffsetMs
      end
      if event and event.judgementBeat and cs then cs.cBeat = event.judgementBeat end
      local success, result = xpcall(function()
        return {updateTaps(self, unpack(args))}
      end, debug.traceback)
      if cs and originalBeat ~= nil then cs.cBeat = originalBeat end
      if gameOptions then gameOptions.inputOffset = originalOffset end
      BBTRenderer.endTapJudgement()
      if not success then error(result, 0) end
      return unpack(result)
    end
  end
  BBT.installedHooks = true
end

function BBT.invalidate(reason, dnf)
  BBT.send('run.invalid', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId, reason = reason, dnf = dnf == true })
end

-- Beatblock routes every gameplay pause input through pauseGame. Blocking at
-- that boundary prevents Escape or a controller from stopping an online chart
-- without changing native pause behavior for offline practice.
function BBT.shouldBlockPause()
  return BBT.context.lobbyId ~= 'offline'
end

function BBT.shouldBlockRetry()
  return BBT.context.lobbyId ~= 'offline'
    and (not BBT.lastLobby or BBT.lastLobby.validityChecksEnabled ~= false)
end

function BBT.onRetry()
  flushScoreDelta(true)
  BBT.invalidate('A retry started during a competitive run', false)
  -- restartLevel swaps in a fresh Game after this hook. Arm the normal
  -- run-ready detector so that attempt receives its own ID and sequence space.
  BBT.context.runId=nil
  BBT.runSequence=0
  BBT.wasRunReady=false
  BBT.scoreDirty=false
end
function BBT.onQuit()
  flushScoreDelta(true)
  BBT.invalidate('Player quit the run', true)
  BBT.send('run.finished', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId, quit = true })
end
local function resultsTotals(forceScore, scoreMax)
  local current = totals()
  if forceScore ~= nil then
    local finalMax = tonumber(scoreMax) or 100
    current.maxHits = finalMax
    current.misses = math.max(0, finalMax - (tonumber(forceScore) or 0))
    current.barelies = 0
  elseif current.maxHits == 0 then
    -- Match Game:goToResults' zero-note guard.
    current.maxHits = 1
  end

  -- Show Results sets exitingLevel during GameManager's event pass. When it
  -- shares a frame with the last tap, Beatblock skips updateTaps and derives
  -- Results from maxHits and penalties anyway. Publish the same terminal
  -- interpretation instead of leaving the host on the previous live divisor.
  current.currentMaxHits = current.maxHits
  current.hits = math.max(current.hits, math.max(0, current.maxHits - current.misses))
  return current
end
function BBT.onResults(forceScore, scoreMax)
  local resultRunId = BBT.context.runId or false
  if BBT.resultsReportedRunId == resultRunId then return end
  -- Results is a terminal scoring boundary even when the final gameplay frame
  -- made no hooked mutation. Always queue this snapshot before run.finished.
  local final = resultsTotals(forceScore, scoreMax)
  if emitScoreDelta(true, final) then BBT.scoreDirty=false end
  emitRenderKeyframe(final, true, forceScore, scoreMax)
  if BBT.send('run.finished', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId }) then
    -- Lovely enters this hook before Beatblock's native `self.results` guard.
    -- Remember a successful terminal publication so duplicate Show Results
    -- events cannot emit another score mutation or completion for this run.
    BBT.resultsReportedRunId = resultRunId
  end
end

function BBT.shouldHoldStart()
  if not BBT.scheduledStartTimeMs then return false end
  return estimatedServerTimeMs() < BBT.scheduledStartTimeMs
end

local function handleCommand(raw)
  local message = decode(raw)
  if not message then return end
  if message.version ~= BBT.protocolVersion then
    BBT.lastError = 'Incompatible runtime protocol. Re-run the Beatblock Online installer.'
    return
  end
  BBT.runtimeLastSeenMs = monotonicMs()
  if message.type ~= 'runtime.launch_status' and message.type ~= 'runtime.error' and message.type ~= 'runtime.disconnected' then
    BBT.companionConnected = true
  end
  if message.type == 'room.context' or message.type == 'lobby.context' then
    BBT.context.lobbyId = message.payload.lobbyId or BBT.context.lobbyId
    BBT.context.lobbyName = message.payload.lobbyName or BBT.context.lobbyName
    BBT.context.playerName = message.payload.playerName or BBT.context.playerName
    BBT.context.userId = message.payload.userId or BBT.context.userId
  elseif message.type == 'runtime.ready' then
    BBT.context.playerName = message.payload.displayName or BBT.context.playerName
    BBT.context.sessionId = message.payload.sessionId or BBT.context.sessionId
    BBT.context.role = message.payload.role or BBT.context.role
    BBT.companionConnected = true
    BBT.connectionStatus = message.payload.connection or 'offline'
    BBT.connected = BBT.connectionStatus == 'hosting' or BBT.connectionStatus == 'connected'
    BBT.runtimeStarting = false
    BBT.runtimeLaunchStatus = nil
    if message.payload.runtimeTimeMs then
      BBT.serverMonotonicOffsetMs = message.payload.runtimeTimeMs - monotonicMs()
      BBT.clockSynchronized = true
      BBT.clockRoundTripMs = 0
    end
  elseif message.type == 'room.start_scheduled' or message.type == 'lobby.start_scheduled' then
    if message.payload.runtimeTimeMs then
      BBT.serverMonotonicOffsetMs = message.payload.runtimeTimeMs - monotonicMs()
      BBT.clockSynchronized = true
    end
    BBT.scheduledStartTimeMs = message.payload.serverStartTimeMs
  elseif message.type == 'room.snapshot' or message.type == 'lobby.snapshot' then
    BBT.lastLobby = message.payload
    BBT.connected = message.payload.lifecycle ~= 'closed' and message.payload.id ~= 'offline'
    if BBT.connected then
      local player=BBT.currentPlayer()
      BBT.connectionStatus=player and player.sessionId==message.payload.hostSessionId and 'hosting' or 'connected'
      if player then BBT.context.sessionId=player.sessionId end
    else
      BBT.connectionStatus='offline'
    end
    BBT.context.lobbyId = message.payload.id or BBT.context.lobbyId
    BBT.context.lobbyName = message.payload.name or BBT.context.lobbyName
    BBT.scheduledStartTimeMs = message.payload.scheduledStartTimeMs
    if message.payload.chart then
      BBT.chartVerified = BBT.localChart ~= nil
        and BBT.localChart.hash == message.payload.chart.hash
        and BBT.localChart.variantName == message.payload.chart.variant
        and BBT.localChart.expectedMaxHits == message.payload.chart.expectedMaxHits
    else
      BBT.chartVerified = false
    end
    if message.payload.participants then
      for _, player in ipairs(message.payload.participants) do
        if player.displayName == BBT.context.playerName then BBT.lastRank = player.rank or BBT.lastRank end
      end
    end
  elseif message.type == 'chart.verification' then
    BBT.chartVerified = message.payload.verified == true
    if BBT.localChart then BBT.localChart.hash = message.payload.hash end
    if not BBT.chartVerified then BBT.lastError = message.payload.reason or 'Chart verification failed' end
  elseif message.type == 'runtime.snapshot' then
    BBT.runtimeSnapshot = message.payload
    BBT.lastLobby = message.payload.room or BBT.lastLobby
    BBT.renderers = message.payload.renderers or {}
    BBT.broadcastPlan = message.payload.broadcastPlan or BBT.broadcastPlan
    BBT.commentatorStatuses = message.payload.commentatorStatuses or {}
    BBT.mirrorEnabled = message.payload.mirrorEnabled == true
    BBT.history = message.payload.history or {}
    BBT.settings = message.payload.settings or BBT.settings
    BBT.connectionStatus = message.payload.connection or BBT.connectionStatus
    BBT.connected = BBT.connectionStatus == 'hosting' or BBT.connectionStatus == 'connected'
    BBT.context.sessionId = message.payload.sessionId or BBT.context.sessionId
    if BBT.settings and BBT.settings.hudEnabled ~= nil then BBT.hudEnabled = BBT.settings.hudEnabled == true end
    BBT.diagnostics = message.payload.diagnostics or BBT.diagnostics
  elseif message.type == 'runtime.heartbeat' then
    BBT.runtimeLastSeenMs = monotonicMs()
  elseif message.type == 'renderer.snapshot' then
    BBT.renderers = message.payload.renderers or BBT.renderers
    BBT.diagnostics = BBT.diagnostics or {}
    BBT.diagnostics.rendererBudgetWarning = message.payload.budgetWarning
  elseif message.type == 'broadcast.plan' then
    if not BBT.broadcastPlan or (message.payload.revision or 0) > (BBT.broadcastPlan.revision or 0) then
      BBT.broadcastPlan = message.payload
    end
  elseif message.type == 'broadcast.revoked' then
    BBT.mirrorEnabled = false
  elseif message.type == 'chart.transfer_offer' then
    BBT.chartTransfer = {
      state = message.payload.containsExecutableContent and 'consent' or 'offer',
      requestId = message.payload.requestId,
      name = message.payload.name,
      size = message.payload.size,
      containsExecutableContent = message.payload.containsExecutableContent == true,
    }
  elseif message.type == 'chart.transfer_progress' then
    BBT.chartTransfer = BBT.chartTransfer or {}
    BBT.chartTransfer.state = 'progress'
    BBT.chartTransfer.percent = message.payload.percent or 0
    BBT.chartTransfer.requestId = message.payload.requestId
  elseif message.type == 'chart.transfer_complete' then
    BBT.chartTransfer = {state='complete',requestId=message.payload.requestId}
  elseif message.type == 'chart.transfer_failed' then
    BBT.chartTransfer = {state='failed'}
    BBT.lastError = message.payload.message or 'Chart transfer failed'
  elseif message.type == 'control.progress' then
    if message.payload.requestId == BBT.pendingRequestId then
      BBT.pendingRequestProgress = message.payload.stage or message.payload.message
    end
  elseif message.type == 'control.ack' then
    if message.payload.requestId == BBT.pendingRequestId then
      BBT.lastCompletedRequestId = BBT.pendingRequestId
      clearPendingRequest()
    end
  elseif message.type == 'runtime.error' or message.type == 'control.error' then
    BBT.lastError = message.payload.message or 'The runtime rejected the command'
    if message.type == 'runtime.error' then BBT.runtimeStarting = false end
    if message.payload.requestId and message.payload.requestId == BBT.pendingRequestId then
      clearPendingRequest()
    end
  elseif message.type == 'runtime.disconnected' then
    BBT.companionConnected = false
    BBT.connected = false
    BBT.runtimeStarting = true
    BBT.connectionStatus = 'reconnecting'
    BBT.runtimeLaunchStatus = message.payload.phase or 'runtime disconnected; reconnecting'
    if BBT.pendingRequestId then
      BBT.lastError = message.payload.message or 'The runtime disconnected before the Online action completed. Reconnecting; please retry.'
      clearPendingRequest()
    end
  elseif message.type == 'runtime.launch_status' then
    BBT.runtimeLaunchStatus = message.payload.phase
  elseif message.type == 'clock.pong' then
    local clientSend = message.payload.clientSendTimeMs
    local companionReceive = message.payload.companionReceiveTimeMs
    local serverSend = message.payload.serverSendTimeMs
    if clientSend and companionReceive and serverSend then
      local roundTrip = math.max(0, companionReceive - clientSend)
      local estimatedServerAtReceipt = serverSend + roundTrip / 2
      BBT.serverMonotonicOffsetMs = estimatedServerAtReceipt - monotonicMs()
      BBT.clockRoundTripMs = roundTrip
      BBT.clockSynchronized = true
    end
  end
end

function BBT.drawCountdown()
  if not BBT.scheduledStartTimeMs then return end
  local remaining = BBT.scheduledStartTimeMs - estimatedServerTimeMs()
  local label = remaining > 0 and tostring(math.max(1, math.ceil(remaining / 1000))) or 'GO'
  love.graphics.setFont(fonts.main)
  color()
  love.graphics.rectangle('fill', project.res.cx - 72, project.res.cy - 42, 144, 84)
  color(1)
  love.graphics.rectangle('line', project.res.cx - 72, project.res.cy - 42, 144, 84)
  love.graphics.printf(label, 0, project.res.cy - 20, project.res.x, 'center')
end

function BBT.drawRaceHud()
  if not BBT.hudEnabled or BBT.context.lobbyId == 'offline' or not BBT.lastLobby then return end
  local player = BBT.currentPlayer()
  love.graphics.setFont(fonts.digitalDisco)
  color()
  love.graphics.rectangle('fill', project.res.x - 174, 8, 166, 45)
  color(1)
  love.graphics.rectangle('line', project.res.x - 174, 8, 166, 45)
  local rank = player and player.rank or BBT.lastRank or 1
  local total = 0
  for _, value in ipairs(BBT.lastLobby.participants or {}) do if value.role ~= 'spectator' then total = total + 1 end end
  local connection = BBT.connected and 'LINK' or 'WARN'
  love.graphics.printf(connection .. '  #' .. tostring(rank) .. '/' .. tostring(math.max(1, total)), project.res.x - 168, 13, 154, 'left')
  if player then love.graphics.printf(string.format('%.2f%%  %+.2f', player.accuracy or 100, (player.accuracy or 100) - 100), project.res.x - 168, 31, 154, 'left') end
end

local function firstScoringBeat()
  if not (cs and type(cs.playEvents) == 'table' and Event and Event.hitCount) then return nil end
  local first = nil
  for _, event in ipairs(cs.playEvents) do
    local beat = nil
    if event.type == 'mine' then
      beat = tonumber(event.time)
    elseif event.type == 'mineHold' then
      beat = (tonumber(event.time) or 0) + (tonumber(event.duration) or 0)
    elseif Event.hitCount[event.type] then
      local ok, count = pcall(Event.hitCount[event.type], event)
      if ok and type(count) == 'number' and count > 0 then beat = tonumber(event.time) end
    end
    if beat and (not first or beat < first) then first = beat end
  end
  -- Decorative/no-score charts still need a deterministic release point.
  return first or tonumber(cs.startBeat) or tonumber(cs.cBeat)
end

function BBT.update(dt)
  if BBT.disabled then return end
  BBT.installHooks()
  if BBTRenderer and BBTRenderer.active then BBTRenderer.update() end
  if not BBT.sessionActive then return end
  if BBT.ipcThread then
    local workerRunning = BBT.ipcThread:isRunning()
    if not BBT.runtimeLaunchStatus then BBT.runtimeLaunchStatus = workerRunning and 'ipc worker running' or 'ipc worker stopped' end
    if not workerRunning then
      local workerError = BBT.ipcThread:getError()
      BBT.runtimeStarting = false
      BBT.runtimeLaunchStatus = 'ipc worker stopped'
      if workerError then BBT.lastError = 'Runtime IPC stopped: ' .. tostring(workerError) end
    end
  end
  local incoming = love.thread.getChannel('bbt_inbound')
  local processed=0
  -- A stalled or backgrounded game may resume to a full bounded backlog.
  -- Amortize JSON/event work so recovery cannot monopolize a render frame.
  while incoming:getCount()>0 and processed<MAX_INBOUND_PER_FRAME do
    handleCommand(incoming:pop())
    processed=processed+1
  end
  if BBT.pendingRequestId and BBT.pendingRequestDeadlineMs and monotonicMs() >= BBT.pendingRequestDeadlineMs then
    local kind = BBT.pendingRequestKind or 'Online action'
    BBT.lastError = kind .. ' did not receive a runtime response. Check the runtime connection and retry.'
    clearPendingRequest()
  end
  BBT.snapshotTimer = BBT.snapshotTimer + dt
  BBT.renderTimer = BBT.renderTimer + dt
  BBT.keyframeTimer = BBT.keyframeTimer + dt
  BBT.heartbeatTimer = (BBT.heartbeatTimer or 0) + dt
  local inGame = cs and cs.level and not cs.results
  if inGame and (not BBT.renderAnchorState or BBT.renderAnchorState.game ~= cs) then
    BBT.renderAnchorState = {game=cs, firstNoteBeat=nil, sent=false}
  elseif not inGame then
    BBT.renderAnchorState = nil
  end
  if inGame and BBT.renderAnchorState and not BBT.renderAnchorState.firstNoteBeat then
    BBT.renderAnchorState.firstNoteBeat = firstScoringBeat()
  end
  if BBT.heartbeatTimer >= 2 then
    BBT.heartbeatTimer = BBT.heartbeatTimer - 2
    BBT.send('client.ping',{instanceId=CLIENT_INSTANCE_ID})
  end
  if BBT.companionConnected and BBT.runtimeLastSeenMs and monotonicMs()-BBT.runtimeLastSeenMs>6000 then
    BBT.companionConnected=false; BBT.connected=false; BBT.connectionStatus='reconnecting'; BBT.runtimeStarting=true
    BBT.runtimeLaunchStatus='runtime heartbeat lost; reconnecting'
    if BBT.pendingRequestId then BBT.lastError='The runtime stopped responding. Reconnecting; please retry.'; clearPendingRequest() end
  end
  -- A renderer needs exact 60 Hz input while playing, but idle Online screens
  -- only need an occasional held-state sample. Avoid serializing and parsing
  -- ninety JSON envelopes per second when no chart is active.
  local renderInterval = inGame and 1 / 60 or 1 / 5
  if BBT.renderTimer >= renderInterval then
    BBT.renderTimer = math.min(BBT.renderTimer - renderInterval, renderInterval)
    local tapMask = 0
    if maininput and maininput.down then
      local ok1, down1 = pcall(maininput.down, maininput, 'tap1')
      local ok2, down2 = pcall(maininput.down, maininput, 'tap2')
      if ok1 and down1 then tapMask = tapMask + 1 end
      if ok2 and down2 then tapMask = tapMask + 2 end
    end
    local flags = 0
    -- Loading a chart creates cs.level while Beatblock is still start-pending
    -- and paused. Publishing that as "playing" lets a delayed renderer start
    -- before the participant's real synchronized first frame.
    local renderPlaying = inGame and not cs.startPending and not cs.paused
    local anchor = BBT.renderAnchorState
    if renderPlaying and anchor and anchor.firstNoteBeat
      and cs.cBeat >= anchor.firstNoteBeat and not anchor.sent then
      local offset = savedata and savedata.options and savedata.options.game
        and tonumber(savedata.options.game.inputOffset) or 0
      anchor.sent = BBT.send('render.anchor', {
        firstNoteBeat=anchor.firstNoteBeat, inputOffsetMs=offset,
      }) == true
    end
    if renderPlaying then flags = flags + 1 end
    if cs and cs.paused then flags = flags + 2 end
    -- These bits are a same-frame fallback for the ordered input.tap event.
    -- Reading input state is non-consuming, so native judgement still receives
    -- the identical edge later in GameManager:updateTaps.
    if maininput and maininput.pressed then
      local ok1, pressed1 = pcall(maininput.pressed, maininput, 'tap1')
      local ok2, pressed2 = pcall(maininput.pressed, maininput, 'tap2')
      local ok3, mouse1 = pcall(maininput.pressed, maininput, 'mouse1')
      local ok4, mouse2 = pcall(maininput.pressed, maininput, 'mouse2')
      local click = savedata and savedata.options and savedata.options.game
        and savedata.options.game.disableClick == false
      if (ok1 and pressed1) or (ok2 and pressed2)
        or (click and ((ok3 and mouse1) or (ok4 and mouse2))) then flags = flags + 4 end
    end
    if maininput and maininput.released then
      local ok1, released1 = pcall(maininput.released, maininput, 'tap1')
      local ok2, released2 = pcall(maininput.released, maininput, 'tap2')
      local ok3, mouse1 = pcall(maininput.released, maininput, 'mouse1')
      -- Match Beatblock's native getTapInputs contract, including its mouse2
      -- press-as-release behavior, so fallback and authoritative judgement agree.
      local ok4, mouse2 = pcall(maininput.pressed, maininput, 'mouse2')
      local click = savedata and savedata.options and savedata.options.game
        and savedata.options.game.disableClick == false
      if (ok1 and released1) or (ok2 and released2)
        or (click and ((ok3 and mouse1) or (ok4 and mouse2))) then flags = flags + 8 end
    end
    BBT.send('render.sample', { beat = cs and cs.cBeat or 0, paddleAngle = cs and cs.p and cs.p.angle or 0, tapMask = tapMask, flags = flags })
  end
  if BBT.keyframeTimer >= 1 then
    BBT.keyframeTimer = math.min(BBT.keyframeTimer - 1, 1)
    -- Results does not retain every live Game counter. Preserve the final
    -- results=true keyframe emitted by goToResults instead of replacing it
    -- with an incomplete idle-state snapshot one second later.
    if inGame then emitRenderKeyframe(nil, false) end
  end
  -- Score mutations remain event-driven. Fifteen gameplay snapshots per second
  -- are enough for overlays; idle screens publish only a two-Hz liveness state.
  local snapshotInterval = inGame and 1 / 15 or 1 / 2
  if BBT.snapshotTimer < snapshotInterval then return end
  BBT.snapshotTimer = math.min(BBT.snapshotTimer - snapshotInterval, snapshotInterval)
  local current = totals()
  local runReady = inGame and current.maxHits > 0
  if runReady and not BBT.wasRunReady then
    BBT.launching = false
    BBT.resultsReportedRunId = nil
    BBT.context.runId = 'run_' .. tostring(os.time()) .. '_' .. tostring(math.random(100000, 999999))
    BBT.runSequence = 0
    BBT.send('run.started', {
      lobbyId = BBT.context.lobbyId,
      runId = BBT.context.runId,
      maxHits = current.maxHits,
      chartHash = BBT.localChart and BBT.localChart.hash or nil,
      variant = BBT.localChart and BBT.localChart.variantName or nil,
    })
  end
  if runReady then flushScoreDelta(false) end
  BBT.wasRunReady = runReady
  BBT.wasInGame = inGame
  local songName = inGame and cs.level.metadata and cs.level.metadata.songName or 'No chart'
  local health = cs and cs.currentHealth or -1
  BBT.send('gameplay.snapshot', {
    state = inGame and (cs.paused and 'paused' or 'playing') or 'idle',
    playerName = BBT.context.playerName,
    songName = songName,
    lobbyName = BBT.context.lobbyName,
    accuracy = accuracy(current),
    combo = current.combo,
    misses = current.misses,
    rank = BBT.lastRank or 1,
    progress = current.maxHits > 0 and math.min(1, current.currentMaxHits / current.maxHits) or 0,
    connected = BBT.connected,
    health = health,
    updatedAtMs = estimatedServerTimeMs(),
  })
end

return BBT
