local BBT = {
  version = '0.3.0-alpha.1',
  protocolVersion = 2,
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
  lastError = nil,
  launching = false,
  clockSynchronized = false,
  wasInGame = false,
  wasRunReady = false,
  sessionActive = false,
  requestSequence = 0,
  hudEnabled = true,
}

local SUPPORTED_GAME_BUILD = 'c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b'

local function nowMs()
  return os.time() * 1000
end

local function monotonicMs()
  return love and love.timer and math.floor(love.timer.getTime() * 1000) or 0
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
  error('Beatblock Together could not find the game JSON encoder')
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

function BBT.send(kind, payload)
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
  love.thread.getChannel('bbt_outbound'):push(encode(message))
end

function BBT.command(kind, payload)
  BBT.lastError = nil
  payload = payload or {}
  BBT.requestSequence = BBT.requestSequence + 1
  local requestId = 'game-' .. tostring(BBT.requestSequence)
  payload.requestId = requestId
  BBT.pendingRequestId = requestId
  BBT.send(kind, payload)
end

-- The native engine is deliberately lazy: normal Beatblock menus never start it.
function BBT.startOnlineRuntime()
  if BBT.sessionActive or (BBTRenderer and BBTRenderer.active) then return end
  BBT.sessionActive = true
  BBT.companionConnected = false
  BBT.runtimeStarting = true
  love.thread.getChannel('bbt_ipc_control'):clear()
  love.thread.getChannel('bbt_outbound'):clear()
  love.thread.getChannel('bbt_inbound'):clear()
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
    BBT.send('client.hello', { clientVersion = BBT.version, gameBuildHash = SUPPORTED_GAME_BUILD, distribution = BBT.distribution, mods = {} })
  else
    BBT.sessionActive = false
    BBT.lastError = 'Could not start the Beatblock Together runtime IPC: ' .. tostring(thread)
  end
end

function BBT.exitOnline()
  if not BBT.sessionActive then return end
  -- Shutdown control must not sit behind stale 60 Hz render samples. Ordered
  -- run journals are already persisted by the runtime; unsent UI telemetry is
  -- expendable once the user explicitly leaves Online.
  love.thread.getChannel('bbt_outbound'):clear()
  BBT.command('runtime.session_end', {})
  BBT.sessionActive = false
  BBT.connected = false
  BBT.companionConnected = false
  BBT.runtimeStarting = false
  BBT.lastLobby = nil
  BBT.context.lobbyId = 'offline'
  love.thread.getChannel('bbt_ipc_control'):push('stop')
end

function BBT.openInstaller()
  local file = io.open(BBT.modPath .. '/installer-path.txt', 'rb')
  local path = file and file:read('*a') or nil
  if file then file:close() end
  if not path or path == '' then BBT.lastError = 'Installer maintenance copy is missing. Download BeatblockTogetherInstaller.exe again.'; return end
  path = path:gsub('[\r\n]+$', '')
  local ok, ffi = pcall(require, 'ffi')
  if not ok then BBT.lastError = 'Windows launcher is unavailable.'; return end
  ffi.cdef[[void* ShellExecuteA(void*, const char*, const char*, const char*, const char*, int);]]
  local result = tonumber(ffi.cast('intptr_t', ffi.C.ShellExecuteA(nil, 'open', path, nil, nil, 1)))
  if not result or result <= 32 then BBT.lastError = 'Windows could not open the installer maintenance copy.' end
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
  BBT.selectingOnlineChart = true
  BBT.chartSelectionMode = mode or 'verify'
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

function BBT.openOfficialSelect(mode)
  BBT.selectingOfficialChart = true
  BBT.chartSelectionMode = mode or 'verify'
  local previous = cs
  local music = previous and previous.menuMusicManager
  cs = bs.load('AtomMap')
  if previous and previous.leave then previous:leave() end
  cs.menuMusicManager = music
  cs:init()
end

function BBT.onOfficialChartSelected(selector, filename, variant)
  if not BBT.selectingOfficialChart then return false end
  local selected = selector.activeQuark
  local level = selected and selected.level
  local variantName = variant and (variant.name or variant.display) or selected and selected.currVariant and (selected.currVariant.name or selected.currVariant.display) or 'Default'
  BBT.localChart = {
    levelPath = filename,
    variantName = variantName,
    variantInfo = variant or (selected and selected.currVariant),
    levelData = level,
    songName = level and level.metadata and level.metadata.songName or filename,
    expectedMaxHits = expectedMaxHits(level) or 1,
    official = true,
  }
  BBT.chartVerified = false
  BBT.selectingOfficialChart = false
  BBT.command((BBT.chartSelectionMode == 'host' or BBT.chartSelectionMode == 'setlist') and 'room.official_chart_select' or 'room.official_chart_verify', {
    chartId = filename,
    songName = BBT.localChart.songName,
    variant = variantName,
    expectedMaxHits = BBT.localChart.expectedMaxHits,
    appendToSetlist = BBT.chartSelectionMode == 'setlist',
  })
  BBT.chartSelectionMode = nil
  local music = selector.menuMusicManager
  if selector.source then selector.source:stop(); selector.source = nil end
  if music then music:clearOnBeatHooks(); music:forceUnmute() end
  cs = bs.load('Online')
  cs.menuMusicManager = music
  cs:init()
  return true
end

local function selectedPackagePath(levelPath)
  local probe = levelPath .. 'manifest.json'
  local real = love.filesystem.getRealDirectory(probe)
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
  local menuMusicManager = selector.menuMusicManager
  local item = selector.menuItems and selector.menuItems[selector.selection]
  if not item then BBT.lastError = 'Beatblock did not expose the selected chart'; return end
  local variantInfo = selector.getVariantInfo and selector:getVariantInfo(item, variantName) or nil
  local packagePath = selectedPackagePath(levelPath)
  BBT.localChart = {
    levelPath = levelPath,
    packagePath = packagePath,
    variantName = variantName or 'Default',
    variantInfo = variantInfo,
    levelData = selector.levelData,
    soundData = selector.preloadSoundData,
    songName = item.name or (item.rawMetadata and item.rawMetadata.songName) or 'Unknown chart',
    expectedMaxHits = expectedMaxHits(selector.levelData),
  }
  BBT.chartVerified = false
  BBT.selectingOnlineChart = false
  if packagePath and BBT.localChart.expectedMaxHits and BBT.localChart.expectedMaxHits > 0 then
    BBT.command((BBT.chartSelectionMode == 'host' or BBT.chartSelectionMode == 'setlist') and 'room.chart_select_request' or 'room.chart_verify_request', {
      path = packagePath,
      levelPath = levelPath,
      songName = BBT.localChart.songName,
      variant = BBT.localChart.variantName,
      expectedMaxHits = BBT.localChart.expectedMaxHits,
      appendToSetlist = BBT.chartSelectionMode == 'setlist',
    })
  elseif packagePath then
    BBT.lastError = 'Chart notes were not preloaded; wait for the song preview and select it again'
  else
    BBT.lastError = 'Could not resolve the selected level package on disk'
  end
  BBT.chartSelectionMode = nil
  if selector.source then selector.source:stop(); selector.source = nil end
  if selector.deletePlayer then selector:deletePlayer() end
  if menuMusicManager then
    menuMusicManager:clearOnBeatHooks()
    menuMusicManager:forceUnmute()
  end
  cs = bs.load('Online')
  cs.menuMusicManager = menuMusicManager
  cs:init()
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
    if log then log('Both Beatblock Together packages are installed. Remove one package before playing.', 'warning') end
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

local function emitScoreDelta()
  local current = totals()
  local songTimeMs = 0
  if cs and cs.source and cs.source.tell then
    local ok, value = pcall(cs.source.tell, cs.source)
    if ok and type(value) == 'number' then songTimeMs = math.floor(value * 1000) end
  end
  BBT.send('run.score_delta', {
    lobbyId = BBT.context.lobbyId,
    runId = BBT.context.runId,
    runSequence = BBT.runSequence,
    progress = current.maxHits > 0 and math.min(1, current.currentMaxHits / current.maxHits) or 0,
    beat = cs and cs.cBeat or 0,
    songTimeMs = songTimeMs,
    totals = current,
  })
  BBT.runSequence = BBT.runSequence + 1
end

function BBT.installHooks()
  if BBT.installedHooks or not GameManager then return end
  local addToScore = GameManager.addToScore
  local handleMiss = GameManager.handleMiss
  local addMineToTotal = GameManager.addMineToTotal
  local getTapInputs = GameManager.getTapInputs
  if not addToScore or not handleMiss or not addMineToTotal then return end
  GameManager.addToScore = function(self, ...)
    local before = totals()
    local result = { addToScore(self, ...) }
    local after = totals()
    if after.hits ~= before.hits or after.barelies ~= before.barelies or after.currentMaxHits ~= before.currentMaxHits then emitScoreDelta() end
    return unpack(result)
  end
  GameManager.handleMiss = function(self, ...)
    local before = totals()
    local result = { handleMiss(self, ...) }
    if totals().misses ~= before.misses then emitScoreDelta() end
    return unpack(result)
  end
  GameManager.addMineToTotal = function(self, ...)
    local before = totals()
    local result = { addMineToTotal(self, ...) }
    if totals().currentMaxHits ~= before.currentMaxHits then emitScoreDelta() end
    return unpack(result)
  end
  if getTapInputs then
    GameManager.getTapInputs = function(self, ...)
      if BBTRenderer and BBTRenderer.active then return BBTRenderer.tapInputs() end
      local pressed, released = getTapInputs(self, ...)
      if pressed or released then
        BBT.send('input.tap', { pressed = pressed == true, released = released == true, beat = cs and cs.cBeat or 0 })
      end
      return pressed, released
    end
  end
  BBT.installedHooks = true
end

function BBT.invalidate(reason, dnf)
  BBT.send('run.invalid', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId, reason = reason, dnf = dnf == true })
end

function BBT.onPause() BBT.invalidate('Pause is not permitted by competitive defaults', false) end
function BBT.onRetry() BBT.invalidate('Retry is not permitted by competitive defaults', true) end
function BBT.onQuit()
  BBT.invalidate('Player quit the competitive run', true)
  BBT.send('run.finished', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId, quit = true })
end
function BBT.onResults() BBT.send('run.finished', { lobbyId = BBT.context.lobbyId, runId = BBT.context.runId }) end

function BBT.shouldHoldStart()
  if not BBT.scheduledStartTimeMs then return false end
  return estimatedServerTimeMs() < BBT.scheduledStartTimeMs
end

local function handleCommand(raw)
  local message = decode(raw)
  if not message then return end
  if message.version ~= BBT.protocolVersion then
    BBT.lastError = 'Incompatible runtime protocol. Re-run the Beatblock Together installer.'
    return
  end
  if message.type ~= 'runtime.launch_status' and message.type ~= 'runtime.error' then
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
    BBT.connected = message.payload.connection == 'hosting' or message.payload.connection == 'connected'
    BBT.runtimeStarting = false
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
    BBT.context.lobbyId = message.payload.id or BBT.context.lobbyId
    BBT.context.lobbyName = message.payload.name or BBT.context.lobbyName
    BBT.scheduledStartTimeMs = message.payload.scheduledStartTimeMs or BBT.scheduledStartTimeMs
    if message.payload.chart then
      BBT.chartVerified = BBT.localChart ~= nil
        and BBT.localChart.hash == message.payload.chart.hash
        and BBT.localChart.variantName == message.payload.chart.variant
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
    BBT.history = message.payload.history or {}
    BBT.settings = message.payload.settings or BBT.settings
    if BBT.settings and BBT.settings.hudEnabled ~= nil then BBT.hudEnabled = BBT.settings.hudEnabled == true end
    BBT.diagnostics = message.payload.diagnostics or BBT.diagnostics
  elseif message.type == 'renderer.snapshot' then
    BBT.renderers = message.payload.renderers or BBT.renderers
    BBT.diagnostics = BBT.diagnostics or {}
    BBT.diagnostics.rendererBudgetWarning = message.payload.budgetWarning
  elseif message.type == 'control.ack' then
    if message.payload.requestId == BBT.pendingRequestId then BBT.pendingRequestId = nil end
  elseif message.type == 'runtime.error' or message.type == 'control.error' then
    BBT.lastError = message.payload.message or 'The runtime rejected the command'
    if message.type == 'runtime.error' then BBT.runtimeStarting = false end
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
  while incoming:getCount() > 0 do handleCommand(incoming:pop()) end
  BBT.snapshotTimer = BBT.snapshotTimer + dt
  BBT.renderTimer = BBT.renderTimer + dt
  BBT.keyframeTimer = BBT.keyframeTimer + dt
  if BBT.renderTimer >= 1 / 60 then
    BBT.renderTimer = 0
    local tapMask = 0
    if maininput and maininput.down then
      local ok1, down1 = pcall(maininput.down, maininput, 'tap1')
      local ok2, down2 = pcall(maininput.down, maininput, 'tap2')
      if ok1 and down1 then tapMask = tapMask + 1 end
      if ok2 and down2 then tapMask = tapMask + 2 end
    end
    local flags = 0
    if cs and cs.level and not cs.results then flags = flags + 1 end
    if cs and cs.paused then flags = flags + 2 end
    BBT.send('render.sample', { beat = cs and cs.cBeat or 0, paddleAngle = cs and cs.p and cs.p.angle or 0, tapMask = tapMask, flags = flags })
  end
  if BBT.keyframeTimer >= 1 then
    BBT.keyframeTimer = 0
    BBT.send('render.keyframe', { beat = cs and cs.cBeat or 0, paddleAngle = cs and cs.p and cs.p.angle or 0, totals = totals(), activeNotes = cs and cs.notes and #cs.notes or 0 })
  end
  if BBT.snapshotTimer < 1 / 30 then return end
  BBT.snapshotTimer = 0
  local current = totals()
  local inGame = cs and cs.level and not cs.results
  local runReady = inGame and current.maxHits > 0
  if runReady and not BBT.wasRunReady then
    BBT.launching = false
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
    updatedAtMs = nowMs(),
  })
end

return BBT
