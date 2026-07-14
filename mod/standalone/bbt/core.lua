local BBT = {
  version = '0.1.0-alpha.1',
  protocolVersion = 1,
  sequence = 0,
  runSequence = 0,
  snapshotTimer = 0,
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
  if json and json.encode then return json.encode(value) end
  if dpf and dpf.json and dpf.json.encode then return dpf.json.encode(value) end
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
  if BBT.disabled then return end
  local message = { version = BBT.protocolVersion, type = kind, sequence = BBT.sequence, timestampMs = math.floor(estimatedServerTimeMs()), payload = payload }
  BBT.sequence = BBT.sequence + 1
  love.thread.getChannel('bbt_outbound'):push(encode(message))
end

function BBT.command(kind, payload)
  BBT.lastError = nil
  BBT.send(kind, payload or {})
end

function BBT.currentPlayer()
  if not BBT.lastLobby or not BBT.lastLobby.players then return nil end
  for _, player in ipairs(BBT.lastLobby.players) do
    if player.userId == BBT.context.userId or player.displayName == BBT.context.playerName then
      return player
    end
  end
  return nil
end

function BBT.isOrganizer()
  return BBT.lastLobby and BBT.lastLobby.organizerId and BBT.currentPlayer()
    and BBT.lastLobby.organizerId == BBT.currentPlayer().userId
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

local function selectedPackagePath(levelPath)
  local probe = levelPath .. 'manifest.json'
  local real = love.filesystem.getRealDirectory(probe)
  if not real then return nil end
  if string.lower(string.sub(real, -4)) == '.zip' then return real end
  local finalCharacter = string.sub(real, -1)
  local separator = (finalCharacter == '/' or finalCharacter == '\\') and '' or '/'
  return real .. separator .. levelPath
end

local function expectedMaxHits(levelData)
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
    BBT.command(BBT.chartSelectionMode == 'host' and 'lobby.chart_select_request' or 'lobby.chart_verify_request', {
      path = packagePath,
      levelPath = levelPath,
      songName = BBT.localChart.songName,
      variant = BBT.localChart.variantName,
      expectedMaxHits = BBT.localChart.expectedMaxHits,
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
  BBT.context.runId = 'run_' .. tostring(os.time()) .. '_' .. tostring(math.random(100000, 999999))
  if bs and bs.states and not bs.states.Online then
    local ok, factory = pcall(require, 'bbt.online_state')
    if ok then bs.states.Online = factory end
  end
  local threadPath = modPath .. '/bbt/ipc_thread.lua'
  local ok, thread = pcall(love.thread.newThread, threadPath)
  if ok then BBT.ipcThread = thread; thread:start() else if log then log('BBT IPC failed: ' .. tostring(thread), 'warning') end end
  BBT.send('client.hello', { clientVersion = BBT.version, gameBuildHash = SUPPORTED_GAME_BUILD, distribution = distribution, mods = {} })
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
  if not message or message.version ~= 1 then return end
  BBT.companionConnected = true
  if message.type == 'lobby.context' then
    BBT.context.lobbyId = message.payload.lobbyId or BBT.context.lobbyId
    BBT.context.lobbyName = message.payload.lobbyName or BBT.context.lobbyName
    BBT.context.playerName = message.payload.playerName or BBT.context.playerName
    BBT.context.userId = message.payload.userId or BBT.context.userId
  elseif message.type == 'companion.ready' then
    BBT.context.playerName = message.payload.displayName or BBT.context.playerName
    BBT.context.userId = message.payload.userId or BBT.context.userId
    BBT.context.role = message.payload.role or BBT.context.role
  elseif message.type == 'lobby.start_scheduled' then
    BBT.scheduledStartTimeMs = message.payload.serverStartTimeMs
  elseif message.type == 'lobby.snapshot' then
    BBT.lastLobby = message.payload
    BBT.scheduledStartTimeMs = message.payload.scheduledStartTimeMs or BBT.scheduledStartTimeMs
    if message.payload.chart then
      BBT.chartVerified = BBT.localChart ~= nil
        and BBT.localChart.hash == message.payload.chart.hash
        and BBT.localChart.variantName == message.payload.chart.variant
    else
      BBT.chartVerified = false
    end
    if message.payload.players then
      for _, player in ipairs(message.payload.players) do
        if player.displayName == BBT.context.playerName then BBT.lastRank = player.rank or BBT.lastRank end
      end
    end
  elseif message.type == 'chart.verification' then
    BBT.chartVerified = message.payload.verified == true
    if BBT.localChart then BBT.localChart.hash = message.payload.hash end
    if not BBT.chartVerified then BBT.lastError = message.payload.reason or 'Chart verification failed' end
  elseif message.type == 'companion.error' then
    BBT.lastError = message.payload.message or 'The companion rejected the command'
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
  elseif message.type == 'gateway.ready' then
    BBT.connected = true
  elseif message.type == 'gateway.disconnected' then
    BBT.connected = false
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
  if BBT.context.lobbyId == 'offline' or not BBT.lastLobby then return end
  local player = BBT.currentPlayer()
  love.graphics.setFont(fonts.digitalDisco)
  color()
  love.graphics.rectangle('fill', project.res.x - 174, 8, 166, 45)
  color(1)
  love.graphics.rectangle('line', project.res.x - 174, 8, 166, 45)
  local rank = player and player.rank or BBT.lastRank or 1
  local total = 0
  for _, value in ipairs(BBT.lastLobby.players or {}) do if not value.spectator then total = total + 1 end end
  love.graphics.printf('ONLINE  #' .. tostring(rank) .. '/' .. tostring(math.max(1, total)), project.res.x - 168, 13, 154, 'left')
  if player then love.graphics.printf(string.format('%.2f%%  %d combo', player.accuracy or 100, player.totals and player.totals.combo or 0), project.res.x - 168, 31, 154, 'left') end
end

function BBT.update(dt)
  if BBT.disabled then return end
  BBT.installHooks()
  local incoming = love.thread.getChannel('bbt_inbound')
  while incoming:getCount() > 0 do handleCommand(incoming:pop()) end
  BBT.snapshotTimer = BBT.snapshotTimer + dt
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
