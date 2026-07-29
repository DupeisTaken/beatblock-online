local autoplay = os.getenv('BBT_RENDERER_AUTOPLAY') == '1'
local rendererSlot = tostring(os.getenv('BBT_RENDERER_STREAM') or 'A'):upper()
if not autoplay and not rendererSlot:match('^[A-D]$') then rendererSlot = 'A' end

local Renderer = {
  active = os.getenv('BBT_RENDERER_FRAME_PATH') ~= nil or autoplay,
  autoplay = autoplay,
  audioOnly = autoplay,
  -- Full is the fidelity-first default: it retains the native backdrop, HUD,
  -- palette/accessibility pass, and chart-authored screen effects.
  mode = os.getenv('BBT_RENDERER_MODE') or 'full',
  framePath = os.getenv('BBT_RENDERER_FRAME_PATH'),
  width = tonumber(os.getenv('BBT_RENDERER_WIDTH')) or 1280,
  height = tonumber(os.getenv('BBT_RENDERER_HEIGHT')) or 720,
  fps = tonumber(os.getenv('BBT_RENDERER_FPS')) or 60,
  audioEnabled = os.getenv('BBT_RENDERER_AUDIO') == '1',
  streamSlot = rendererSlot,
  windowTitle = autoplay and 'Beatblock Online Autoplay'
    or ('Beatblock Online Renderer ' .. rendererSlot),
  sequence = 0, captureSequence = 0, lastInputSequence = 0, tapMask = 0,
  lastScoreSequence = 0, sourceAccuracy = nil, sourceOffset = 0,
  sourceScore = nil, resultsReady = false,
  readbackPending = {false,false}, readbackRequests = {nil,nil}, readbackTickets = {0,0},
  readbackStartedAt = {nil,nil},
  playing = false, hasInput = false, droppedFrames = 0, nextFrameAt = nil,
  previousAngle = nil, captureEnabled = false, inputOffsetMs = 0,
  tapQueue = {}, currentTapEvent = nil, seedPaddle = false,
  pendingAudioSync = false, lastAudioCorrectionAt = -math.huge,
  lastAppliedPaddleSequence = nil, paddleCumulativeAngle = nil,
}
Renderer.statePath = os.getenv('BBT_RENDERER_STATE_PATH')
  or (Renderer.framePath or ''):gsub('%.bbtframe$', '.bbtstate')
Renderer.scorePath = os.getenv('BBT_RENDERER_SCORE_PATH')
  or (Renderer.framePath or ''):gsub('%.bbtframe$', '.bbtscore')
Renderer.errorPath = os.getenv('BBT_RENDERER_ERROR_PATH')
  or (Renderer.framePath or ''):gsub('%.bbtframe$', '.bbterror')

local ok, ffi = pcall(require, 'ffi')
if ok then ffi.cdef[[
  typedef void* HANDLE; typedef unsigned long DWORD; typedef int BOOL;
  HANDLE CreateFileW(const wchar_t*, DWORD, DWORD, void*, DWORD, DWORD, HANDLE);
  HANDLE CreateFileMappingA(HANDLE, void*, DWORD, DWORD, DWORD, const char*);
  void* MapViewOfFile(HANDLE, DWORD, DWORD, DWORD, size_t);
  BOOL UnmapViewOfFile(const void*); BOOL CloseHandle(HANDLE);
  int MultiByteToWideChar(unsigned int, DWORD, const char*, int, wchar_t*, int);
  void* GetActiveWindow(void); int ShowWindow(void*, int);
  void** SDL_GetWindows(int*); bool SDL_MinimizeWindow(void*); void SDL_free(void*);
]] end

local sdl = nil
if ok then
  local loaded, library = pcall(ffi.load, 'SDL3')
  if loaded then sdl = library end
end

local function wide(value)
  local length = ffi.C.MultiByteToWideChar(65001, 0, value, #value, nil, 0)
  if length <= 0 then return nil end
  local result = ffi.new('wchar_t[?]', length + 1)
  if ffi.C.MultiByteToWideChar(65001, 0, value, #value, result, length) <= 0 then return nil end
  return result
end

local function mapFile(path, size)
  if not ok or not path or path == '' then return nil end
  local pathWide = wide(path)
  if not pathWide then return nil end
  local handle = ffi.C.CreateFileW(pathWide, 0xC0000000, 3, nil, 3, 0, nil)
  if handle == ffi.cast('HANDLE', -1) then return nil end
  local mapping = ffi.C.CreateFileMappingA(handle, nil, 0x04, 0, size, nil)
  if mapping == nil then ffi.C.CloseHandle(handle); return nil end
  local pointer = ffi.C.MapViewOfFile(mapping, 0x000F001F, 0, 0, size)
  if pointer == nil then ffi.C.CloseHandle(mapping); ffi.C.CloseHandle(handle); return nil end
  return {handle=handle, mapping=mapping, pointer=ffi.cast('uint8_t*', pointer)}
end

local function unmapFile(mapped)
  if not ok or not mapped then return end
  if mapped.pointer ~= nil then ffi.C.UnmapViewOfFile(mapped.pointer); mapped.pointer = nil end
  if mapped.mapping ~= nil then ffi.C.CloseHandle(mapped.mapping); mapped.mapping = nil end
  if mapped.handle ~= nil then ffi.C.CloseHandle(mapped.handle); mapped.handle = nil end
end

local function minimizeRendererWindow()
  -- GetActiveWindow is often nil when the child was launched behind OBS. Ask
  -- SDL for this process's windows first. OBS Application Audio Capture ignores
  -- fully hidden windows, but explicitly includes minimized windows, so the
  -- child must remain discoverable without covering the player's desktop.
  local minimized = false
  if sdl then
    local count = ffi.new('int[1]')
    local windows = sdl.SDL_GetWindows(count)
    if windows ~= nil then
      for index = 0, count[0] - 1 do
        if windows[index] ~= nil and sdl.SDL_MinimizeWindow(windows[index]) then minimized = true end
      end
      sdl.SDL_free(windows)
    end
  end
  if not minimized and love and love.window and love.window.minimize then
    minimized = pcall(love.window.minimize)
  end
  if not minimized then
    local window = ffi.C.GetActiveWindow()
    if window ~= nil then ffi.C.ShowWindow(window, 6) end
  end
end

-- Renderer windows are hidden, so Beatblock's modal crash reporter is not a
-- usable diagnostic surface. Persist every fatal/capture error for the runtime
-- dashboard before handing control back to the game's normal error handler.
local function reportError(message)
  Renderer.captureError = tostring(message)
  if log then log('Renderer failed: ' .. Renderer.captureError, 'error') end
  local file = io.open(Renderer.errorPath, 'wb')
  if file then
    file:write(Renderer.captureError)
    file:close()
  end
end

function Renderer.shutdown()
  if Renderer.audioEnabled and love and love.audio and love.audio.setVolume then
    pcall(love.audio.setVolume, 0)
  end
  unmapFile(Renderer.inputs)
  unmapFile(Renderer.scores)
  unmapFile(Renderer.frames)
  -- Drop pending GPU readbacks and canvases when initialization fails. Normal
  -- renderer termination is process-scoped, but this path must not retain
  -- graphics resources while the hidden child is waiting to exit.
  Renderer.readbackPending = {false,false}
  Renderer.readbackRequests = {nil,nil}
  Renderer.readbackStartedAt = {nil,nil}
  Renderer.inputs, Renderer.scores, Renderer.frames, Renderer.outputs = nil, nil, nil, nil
  Renderer.active = false
end

local function failInitialization(message)
  Renderer.initializationError = message
  Renderer.shutdown()
  if love and love.event and love.event.quit then love.event.quit(1) else os.exit(1) end
  return Renderer
end

function Renderer.init()
  if not Renderer.active then return Renderer end
  if love and love.errorhandler and not Renderer.originalErrorHandler then
    Renderer.originalErrorHandler = love.errorhandler
    love.errorhandler = function(message)
      reportError('renderer crashed:\n' .. debug.traceback(tostring(message or ''), 2))
      return Renderer.originalErrorHandler(message)
    end
  end
  if not ok then return failInitialization('LuaJIT FFI is unavailable') end
  if not Renderer.audioOnly and (
    Renderer.width ~= math.floor(Renderer.width)
    or Renderer.height ~= math.floor(Renderer.height)
    or Renderer.width < 320
    or Renderer.width > 1920
    or Renderer.height < 180
    or Renderer.height > 1080
  ) then
    return failInitialization('renderer dimensions are outside the supported range')
  end
  if not Renderer.audioOnly and Renderer.fps ~= 30 and Renderer.fps ~= 60 then
    return failInitialization('renderer FPS must be 30 or 60')
  end
  if Renderer.mode ~= 'clean' and Renderer.mode ~= 'full' then
    return failInitialization('renderer mode must be clean or full')
  end
  if not (love and love.window and love.window.setTitle)
    or not pcall(love.window.setTitle, Renderer.windowTitle) then
    return failInitialization('renderer window title could not be assigned')
  end
  Renderer.inputs = mapFile(Renderer.statePath, 32)
  if not Renderer.inputs then
    return failInitialization('renderer shared-memory files could not be mapped')
  end
  if Renderer.audioOnly then
    minimizeRendererWindow()
    _G.BBTRenderer = Renderer
    return Renderer
  end
  Renderer.frameSize = Renderer.width * Renderer.height * 4
  Renderer.frameInterval = 1 / math.max(1, Renderer.fps)
  Renderer.frames = mapFile(Renderer.framePath, 64 + 1920 * 1080 * 4 * 3)
  Renderer.scores = mapFile(Renderer.scorePath, 48)
  if not Renderer.frames or not Renderer.scores then
    return failInitialization('renderer video shared-memory files could not be mapped')
  end
  -- Two canvases keep one GPU readback from forcing an avoidable 60 Hz drop.
  -- Shared-memory dimensions are physical pixels, so never inherit Windows'
  -- per-monitor DPI scale (for example 1280x720 becoming a 1600x900 readback).
  local canvasOk, first, second = pcall(function()
    return love.graphics.newCanvas(Renderer.width, Renderer.height, {
      format='rgba8', readable=true, dpiscale=1,
    }), love.graphics.newCanvas(Renderer.width, Renderer.height, {
      format='rgba8', readable=true, dpiscale=1,
    })
  end)
  if not canvasOk or not first or not second then
    return failInitialization('renderer canvases could not be allocated')
  end
  Renderer.outputs = {first, second}
  minimizeRendererWindow()
  _G.BBTRenderer = Renderer
  return Renderer
end

function Renderer.clearPreviousState()
  -- Renderer startup bypasses SongSelect's grow-transition callback, where the
  -- normal game flow deletes its menu Player and other retained UI entities.
  -- Menu:leave intentionally preserves MenuBackground for sibling menus, so a
  -- direct jump to Game must establish the native gameplay ownership boundary.
  if em and em.clear then em.clear() end
  if flux and flux.tweens and flux.remove then
    while #flux.tweens > 0 do flux.remove(#flux.tweens) end
  end
end

function Renderer.useDefaultCostume()
  if not (savedata and savedata.costumes) then return end
  -- Renderer profiles are disposable and must never inherit a host save's
  -- selected costume. `none` is Beatblock's built-in default Cranky look.
  savedata.costumes.currentCostume='none'
end

function Renderer.start()
  if not Renderer.active then return end
  local chart, variant = os.getenv('BBT_RENDERER_CHART'), os.getenv('BBT_RENDERER_VARIANT')
  if not chart or chart == '' then return end
  if not string.match(chart, '[/\\]$') then chart = chart .. '/' end
  local variantInfo = nil
  if variant and variant ~= '' and dpf and dpf.loadJson then
    local loaded, manifest = pcall(dpf.loadJson, chart .. 'manifest.json')
    if loaded and manifest and manifest.variants then
      for _, candidate in ipairs(manifest.variants) do
        if string.lower(tostring(candidate.name or '')) == string.lower(variant) then
          variantInfo = candidate
          break
        end
      end
    end
  end
  local previous = cs
  -- LÖVE's Windows save directory ignores the child APPDATA override. The
  -- renderer is disposable, so prevent it from writing the player's save when
  -- leaving menu/game states.
  if sdfunc and sdfunc.save then
    Renderer.originalSave = Renderer.originalSave or sdfunc.save
    sdfunc.save = function() end
  end
  -- Results writes played-level data directly instead of going through
  -- sdfunc.save. A renderer is a disposable replay process and must never
  -- mutate the player's progress or unlock files.
  if dpf and dpf.saveJson then
    Renderer.originalDpfSaveJson = Renderer.originalDpfSaveJson or dpf.saveJson
    dpf.saveJson = function() end
  end
  if savedata and savedata.options and savedata.options.game then
    -- The remote angle already contains circle-snap and April Fools offsets.
    -- Reapplying either in the hidden child produces a second, false motion.
    savedata.options.game.forceMouseKeyboard = true
    savedata.options.game.circleSnap = 'disabled'
  end
  if savedata and savedata.options and savedata.options.aprilFools then
    savedata.options.aprilFools.randomPaddleOffset = false
  end
  Renderer.installAutoplayHooks()
  Renderer.useDefaultCostume()
  -- Beatblock's love.focus handler may already have globally muted this
  -- background process. Disable future focus-loss muting and restore the
  -- renderer's master volume before the Play Song fallback creates its source.
  local audioOptions = savedata and savedata.options and savedata.options.audio
  if Renderer.audioEnabled then
    if audioOptions then audioOptions.muteOnFocusLoss = false end
    if love and love.audio and love.audio.setVolume then love.audio.setVolume(1) end
  elseif audioOptions then
    for key, value in pairs(audioOptions) do
      if type(value) == 'number' then audioOptions[key] = 0 end
    end
  end
  -- Beatblock's background loader reads the global cLevel even though Game:init
  -- also receives the chart path. Mirror the native Freeplay launch contract.
  cLevel = chart
  cs = bs.load('Game')
  if GameManager and GameManager.transferStateData and previous then GameManager:transferStateData(cs, previous) end
  if previous and previous.leave then previous:leave() end
  Renderer.clearPreviousState()
  cs.bbtRenderer = true
  -- Freeplay normally supplies a table with `path` and `data`. An empty table
  -- preserves that shape and lets Beatblock load the song without attempting
  -- to index a boolean sentinel.
  cs:init(chart, variantInfo, nil, {})
  if cs.source and cs.source.setVolume then
    cs.source:setVolume(Renderer.audioEnabled and 1 or 0)
  end
end

function Renderer.canonicalAutoplayCollision(note)
  local name = note and note.name
  return name ~= 'mine' and name ~= 'mineHold'
end

function Renderer.installAutoplayHooks()
  if not Renderer.autoplay or Renderer.autoplayHooksInstalled then return end
  local accessibility = savedata and savedata.options and savedata.options.accessibility
  if accessibility then
    -- Beatblock's native Tap and Side behaviors already implement a zero-offset
    -- automatic judgement path. Reusing them preserves tap/hold completion and
    -- guarantees exactly one normal positive hitsound per scoring opportunity.
    accessibility.taps = 'auto'
    accessibility.sides = 'auto'
  end
  if Block and Block.checkTouchingPaddle then
    Block.checkTouchingPaddle = function(note)
      if Renderer.canonicalAutoplayCollision(note) then return true, false end
      return false, false
    end
  end
  if MineHold and MineHold.checkTouchingPaddle then
    MineHold.checkTouchingPaddle = function() return false end
  end
  Renderer.autoplayHooksInstalled = true
end

function Renderer.applySourceScore(target)
  local score=Renderer.sourceScore
  if not score or not target then return end
  target.hits=score.hits
  target.misses=score.misses
  target.barelies=score.barelies
  target.combo=score.combo
  target.maxCombo=score.maxCombo
  target.currentMaxHits=score.currentMaxHits
  target.maxHits=score.maxHits
end

local function applySourceResults(target)
  if not target or not Renderer.sourceScore then return end
  Renderer.applySourceScore(target)
  target.offset=Renderer.sourceOffset
  target.pctGrade=Renderer.sourceAccuracy
  target.pctGradeRender=string.format('%.2f',Renderer.sourceAccuracy)
  if GameManager and GameManager.gradeCalc then
    target.lGrade,target.lGradePM=GameManager:gradeCalc(Renderer.sourceAccuracy)
    if target.barelies+target.misses==1 then
      target.lGrade='almost'
      target.lGradePM='none'
    end
  end
end

function Renderer.shouldShowResults()
  return Renderer.resultsReady
end

local function enterSourceResults()
  if not Renderer.resultsReady or not cs then return end
  if cs.name=='Game' and cs.goToResults and not cs.results and not cs.startPending and cs.vfx then
    -- Install the final source totals before Beatblock transfers Game state into
    -- Results. A relaunched child can briefly observe a score page before Game
    -- finishes threaded initialization; Game:leave requires vfx to exist.
    Renderer.applySourceScore(cs)
    cs:goToResults()
  end
  if cs and cs.name=='Results' then applySourceResults(cs) end
end

local function updateSourceScore()
  if not Renderer.scores then return end
  local sequence=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer)[0])
  if sequence==0 then
    if Renderer.lastScoreSequence~=0 then
      -- The runtime zeros this page between runs. Drop every prior source field
      -- so the next run cannot inherit an old HUD or final Results marker, and
      -- allow its commit counter to safely restart from one.
      Renderer.lastScoreSequence=0
      Renderer.sourceAccuracy=nil
      Renderer.sourceOffset=0
      Renderer.sourceScore=nil
      Renderer.resultsReady=false
    end
    return
  end
  if sequence==Renderer.lastScoreSequence then return end
  local flags=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+4)[0])
  local sourceAccuracy=tonumber(ffi.cast('float*',Renderer.scores.pointer+8)[0])
  local sourceOffset=tonumber(ffi.cast('float*',Renderer.scores.pointer+12)[0])
  local score={
    hits=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+16)[0]),
    misses=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+20)[0]),
    barelies=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+24)[0]),
    combo=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+28)[0]),
    maxCombo=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+32)[0]),
    currentMaxHits=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+36)[0]),
    maxHits=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+40)[0]),
    mineHits=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer+44)[0]),
  }
  -- The score writer commits sequence last. Do not combine fields from two
  -- source keyframes if the runtime advances the slot during this read.
  if sequence~=tonumber(ffi.cast('uint32_t*',Renderer.scores.pointer)[0]) then return end
  if sourceAccuracy~=sourceAccuracy or sourceAccuracy<0 or sourceAccuracy>100
    or sourceOffset~=sourceOffset or sourceOffset==math.huge or sourceOffset==-math.huge then return end
  Renderer.lastScoreSequence=sequence
  Renderer.sourceAccuracy=sourceAccuracy
  Renderer.sourceOffset=sourceOffset
  Renderer.sourceScore=score
  Renderer.resultsReady=flags%2==1
  enterSourceResults()
end

function Renderer.update()
  if not Renderer.active or not Renderer.inputs then return end
  updateSourceScore()
  local sequence = tonumber(ffi.cast('uint32_t*', Renderer.inputs.pointer + 8)[0])
  if sequence == 0 then return end
  local beat = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 20)[0])
  local angle = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 24)[0])
  local tapMask = tonumber(ffi.cast('uint16_t*', Renderer.inputs.pointer + 28)[0])
  local flags = tonumber(ffi.cast('uint16_t*', Renderer.inputs.pointer + 30)[0])
  local judgementBeat = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 12)[0])
  local inputOffsetMs = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 16)[0])
  -- The writer commits sequence last. Reject a sample that changed while its
  -- remaining fields were being copied instead of steering from a torn frame.
  if sequence ~= tonumber(ffi.cast('uint32_t*', Renderer.inputs.pointer + 8)[0]) then
    Renderer.steerPaddle()
    return
  end
  if beat ~= beat or angle ~= angle or beat == math.huge or beat == -math.huge
    or angle == math.huge or angle == -math.huge then
    Renderer.steerPaddle()
    return
  end
  if sequence ~= Renderer.lastInputSequence then
    Renderer.lastInputSequence = sequence
    Renderer.tapMask = tapMask
    Renderer.beat = beat
    Renderer.previousAngle = Renderer.angle
    Renderer.angle = angle % 360
    Renderer.playing = flags % 2 == 1
    Renderer.captureEnabled = math.floor(flags / 16) % 2 == 1
    Renderer.inputOffsetMs = inputOffsetMs == inputOffsetMs and inputOffsetMs or 0
    Renderer.hasInput = true
    if math.floor(flags / 32) % 2 == 1 then
      Renderer.seedPaddle = true
      Renderer.pendingAudioSync = true
      Renderer.tapQueue = {}
      Renderer.currentTapEvent = nil
      -- A release/source-change starts a new authoritative motion history.
      -- Do not join the new participant to the previous paddle's angle delta.
      Renderer.previousAngle = Renderer.angle
      Renderer.lastAppliedPaddleSequence = nil
      Renderer.paddleCumulativeAngle = Renderer.angle
    end
    local pressed = math.floor(flags / 64) % 2 == 1
    local released = math.floor(flags / 128) % 2 == 1
    if pressed or released then
      if #Renderer.tapQueue >= 256 then table.remove(Renderer.tapQueue, 1) end
      Renderer.tapQueue[#Renderer.tapQueue + 1] = {
        pressed=pressed,
        released=released,
        judgementBeat=judgementBeat == judgementBeat and judgementBeat or nil,
      }
    end
  end
  -- Gamestate updates the hidden OS mouse before this hook on every frame, even
  -- when no new network sample arrived. Rebuild the same absolute vector each
  -- frame so Player:update never sees the hidden window's upper-left position.
  Renderer.steerPaddle()
end

function Renderer.steerPaddle()
  if not Renderer.hasInput or not cs or not cs.p or not Renderer.angle then return end
  local player = cs.p
  local angle = Renderer.angle
  if Renderer.seedPaddle then
    -- Seed exactly once at the first-note barrier. Subsequent movement flows
    -- through Player:update, retaining native angle caps, history and feedback.
    player.angle = angle
    player.anglePrevFrame = angle
    player.angleDelta = 0
    player.cumulativeAngle = angle
    Renderer.seedPaddle = false
  end
  local radius = math.max(tonumber(player.radius) or 0, 64)
  local radians = math.rad(angle - 90)
  local circleX, circleY = math.cos(radians) * radius, math.sin(radians) * radius
  player.circleX, player.circleY = circleX, circleY
  player.snapX, player.snapY = circleX, circleY
  if mouse then
    mouse.circleSnap = 'disabled'
    mouse.rx = (player.x or 0) + circleX
    mouse.ry = (player.y or 0) + circleY
    mouse.dx, mouse.dy = 0, 0
  end
end

function Renderer.applyPaddleState(player)
  if not Renderer.hasInput or not player or not Renderer.angle then return end

  -- `render.sample.paddleAngle` is captured after the source Player has already
  -- applied circle snap, controller offsets, and its native per-frame cap. If
  -- the hidden Player caps that value again, one skipped/repeated 60 Hz sample
  -- leaves its collision angle behind the source and turns hits into persistent
  -- miss ghosts. Install the sampled state after Player:update and before the
  -- higher-layer notes update instead.
  local angle = Renderer.angle
  local advanced = Renderer.lastAppliedPaddleSequence ~= Renderer.lastInputSequence
  local previous = advanced and (Renderer.previousAngle or angle) or angle
  local delta = 0
  if advanced then
    if helpers and helpers.angdelta then
      delta = helpers.angdelta(previous, angle)
    else
      delta = (angle - previous + 180) % 360 - 180
    end
    Renderer.paddleCumulativeAngle = (Renderer.paddleCumulativeAngle or previous) + delta
    Renderer.lastAppliedPaddleSequence = Renderer.lastInputSequence
  end

  player.anglePrevFrame = previous
  player.angle = angle
  player.angleDelta = delta
  player.cumulativeAngle = Renderer.paddleCumulativeAngle or angle
  Renderer.steerPaddle()
end

-- This hook runs inside GameManager after its local audio clock assignment but
-- before chart events, taps, notes and eases. The remote beat therefore becomes
-- the sole simulation clock instead of a cosmetic correction after the fact.
function Renderer.applyClock()
  if Renderer.hasInput and Renderer.playing and cs then cs.cBeat = Renderer.beat end
end

function Renderer.afterGameUpdate()
  -- Native simulation remains responsible for visual feedback, but its score
  -- mutations can diverge when a packet is skipped. Restore the delayed source
  -- totals at the final gameplay boundary before HUD drawing and Results.
  Renderer.applySourceScore(cs)
  if not Renderer.hasInput or not cs or not cs.source then return end
  if not (cs.source.getBeat and cs.source.setBeat) then return end
  local now = love.timer.getTime()
  local okBeat, sourceBeat = pcall(cs.source.getBeat, cs.source)
  local materialDrift = okBeat and type(sourceBeat) == 'number'
    and math.abs(sourceBeat - Renderer.beat) > .20
    and now - Renderer.lastAudioCorrectionAt >= 2
  if Renderer.pendingAudioSync or materialDrift then
    local corrected = pcall(cs.source.setBeat, cs.source, Renderer.beat)
    if corrected then
      Renderer.pendingAudioSync = false
      Renderer.lastAudioCorrectionAt = now
    end
  end
end

function Renderer.shouldHold()
  -- Cached delayed samples warm the native game before OBS is allowed to see
  -- it. Holding only until a playing sample arrives preserves chart-authored
  -- pre-roll events while captureEnabled remains the first-note output gate.
  return Renderer.active and (not Renderer.hasInput or not Renderer.playing)
end

function Renderer.shouldFreezeSimulation()
  -- Returning from Game:update alone is not a complete pause: Beatblock's
  -- outer Gamestate loop still advances flux eases and every Entity. Freeze
  -- those systems after threaded preload so hidden Player canvases and VFX do
  -- not age for an arbitrary amount of time while waiting for source input.
  return Renderer.shouldHold() and cs and cs.name == 'Game' and not cs.startPending
end

function Renderer.beginTapJudgement()
  Renderer.currentTapEvent = table.remove(Renderer.tapQueue, 1)
  return Renderer.currentTapEvent
end

function Renderer.tapInputs()
  local event = Renderer.currentTapEvent
  return event and event.pressed or false, event and event.released or false
end

function Renderer.endTapJudgement()
  Renderer.currentTapEvent = nil
end

local function drawSource(source, finalShader)
  if not source or not source.getDimensions then error('renderer capture source is unavailable') end
  local sourceWidth, sourceHeight = source:getDimensions()
  if not sourceWidth or sourceWidth <= 0 or not sourceHeight or sourceHeight <= 0 then
    error('renderer capture source has invalid dimensions')
  end
  love.graphics.push('all')
  local success, message = xpcall(function()
    -- Beatblock leaves draw state behind for the rest of its own composition.
    -- The source is already the final shaded frame. Disable every mask/clip
    -- that can survive a chart draw before clearing the reused output canvas;
    -- otherwise untouched pixels from older frames produce dithered ghosts.
    love.graphics.origin()
    love.graphics.setScissor()
    love.graphics.setStencilTest()
    if love.graphics.setDepthMode then love.graphics.setDepthMode() end
    if love.graphics.setColorMask then love.graphics.setColorMask(true,true,true,true) end
    love.graphics.setShader(finalShader)
    love.graphics.setBlendMode('alpha', 'alphamultiply')
    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.clear(0, 0, 0, 1)
    local scale = math.min(Renderer.width / sourceWidth, Renderer.height / sourceHeight)
    local x = (Renderer.width - sourceWidth * scale) / 2
    local y = (Renderer.height - sourceHeight * scale) / 2
    love.graphics.draw(source, x, y, 0, scale, scale)
  end, debug.traceback)
  love.graphics.pop()
  if not success then error(message, 0) end
end

local function copyFrame(data, readbackSlot, ticket)
  if Renderer.readbackTickets[readbackSlot] ~= ticket then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    return
  end
  Renderer.readbackPending[readbackSlot] = false
  Renderer.readbackRequests[readbackSlot] = nil
  Renderer.readbackStartedAt[readbackSlot] = nil
  if not Renderer.frames then return end
  if not data then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
    return
  end
  local pointer = data.getFFIPointer and data:getFFIPointer() or nil
  local dataSize = data.getSize and data:getSize() or 0
  -- Never commit a short readback over a previously valid ring slot: OBS would
  -- otherwise receive a frame made from new leading bytes and stale trailing
  -- bytes while its sequence check still appeared healthy.
  if not pointer or dataSize ~= Renderer.frameSize then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
    return
  end
  -- Async callbacks are permitted to complete out of order. Never publish an
  -- older capture over a newer one already visible to OBS.
  if ticket <= Renderer.sequence then Renderer.droppedFrames = Renderer.droppedFrames + 1; return end
  Renderer.sequence = ticket
  local index = Renderer.sequence % 3
  ffi.copy(Renderer.frames.pointer + 64 + index * Renderer.frameSize, pointer, Renderer.frameSize)
  ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
  -- The aligned sequence is the commit marker and is written after all pixels.
  ffi.cast('uint64_t*', Renderer.frames.pointer + 32)[0] = Renderer.sequence
end

function Renderer.reclaimStalledReadbacks(now)
  now=now or love.timer.getTime()
  for slot=1,2 do
    local started=Renderer.readbackStartedAt[slot]
    if Renderer.readbackPending[slot] and started and now-started>=1 then
      Renderer.readbackRequests[slot]=nil
      Renderer.readbackPending[slot]=false
      Renderer.readbackStartedAt[slot]=nil
      -- Invalidate the abandoned request before this canvas is reused.
      Renderer.readbackTickets[slot]=nil
      Renderer.droppedFrames=Renderer.droppedFrames+1
      if Renderer.frames then
        ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0]=Renderer.droppedFrames
      end
    end
  end
end

local function finishReadbacks()
  Renderer.reclaimStalledReadbacks(love.timer.getTime())
  for slot = 1, 2 do
    local request = Renderer.readbackRequests[slot]
    if request then
      local updated = pcall(request.update, request)
      local checked, complete = pcall(request.isComplete, request)
      if not updated or not checked then
        Renderer.readbackRequests[slot] = nil
        Renderer.readbackPending[slot] = false
        Renderer.readbackStartedAt[slot] = nil
        Renderer.droppedFrames = Renderer.droppedFrames + 1
      elseif complete then
        local failed = request:hasError()
        local data = not failed and request:getImageData() or nil
        if failed or not data then
          Renderer.readbackRequests[slot] = nil
          Renderer.readbackPending[slot] = false
          Renderer.readbackStartedAt[slot] = nil
          Renderer.droppedFrames = Renderer.droppedFrames + 1
        else
          copyFrame(data, slot, Renderer.readbackTickets[slot])
        end
      end
    end
  end
end

function Renderer.capture(cleanSource, shadedSource, finalShader)
  if Renderer.audioOnly or not Renderer.active or not Renderer.frames or not Renderer.captureEnabled then return end
  local now = love.timer.getTime()
  -- LÖVE 12 returns a GraphicsReadback object; it does not accept a callback.
  -- Poll completed requests before reserving an output canvas for this frame.
  finishReadbacks()
  Renderer.nextFrameAt = Renderer.nextFrameAt or now
  if now + .0005 < Renderer.nextFrameAt then return end
  if now - Renderer.nextFrameAt >= Renderer.frameInterval then
    local missed = math.floor((now - Renderer.nextFrameAt) / Renderer.frameInterval)
    Renderer.droppedFrames = Renderer.droppedFrames + missed
    Renderer.nextFrameAt = Renderer.nextFrameAt + (missed + 1) * Renderer.frameInterval
  else
    Renderer.nextFrameAt = Renderer.nextFrameAt + Renderer.frameInterval
  end
  local readbackSlot = not Renderer.readbackPending[1] and 1 or (not Renderer.readbackPending[2] and 2 or nil)
  if not readbackSlot then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
    return
  end
  local output=Renderer.outputs[readbackSlot]
  -- Clean mode is the real uncomposited gameplay canvas. Full mode receives
  -- shuv.canvasShaded after Beatblock has applied its palette/accessibility
  -- shader and every in-state HUD layer. Capturing raw shuv.canvas exposed its
  -- red palette-index artwork as a full-screen mask instead of the player view.
  local source = Renderer.mode == 'full' and shadedSource or cleanSource
  output:renderTo(function()
    -- `capturePlayerView` selects the appropriate final shader for both modes:
    -- chromatic composition in Full, palette/accessibility mapping in Clean.
    drawSource(source, finalShader)
  end)
  Renderer.captureSequence = Renderer.captureSequence + 1
  local ticket=Renderer.captureSequence
  Renderer.readbackPending[readbackSlot] = true
  Renderer.readbackTickets[readbackSlot] = ticket
  Renderer.readbackStartedAt[readbackSlot] = now
  if love.graphics.readbackTextureAsync then
    local success, request = pcall(love.graphics.readbackTextureAsync, output)
    if success and request then
      Renderer.readbackRequests[readbackSlot] = request
      return
    end
  end
  local success, data = pcall(love.graphics.readbackTexture, output)
  if success then copyFrame(data,readbackSlot,ticket) else
    Renderer.readbackPending[readbackSlot] = false
    Renderer.readbackStartedAt[readbackSlot] = nil
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
  end
end

function Renderer.captureSafe(cleanSource, shadedSource, finalShader)
  if Renderer.captureError then return end
  local success, message = xpcall(function()
    Renderer.capture(cleanSource, shadedSource, finalShader)
  end, debug.traceback)
  if success then return end
  reportError('renderer capture failed:\n' .. tostring(message))
end

function Renderer.capturePlayerView(cleanSource, shadedSource)
  -- shuv.finish applies chromatic aberration only while drawing its shaded
  -- canvas to the window. Reapply that final screen-space shader to the OBS
  -- output so Full mode follows the player's view instead of stopping one
  -- post-processing pass early.
  local finalShader=nil
  local chromatic=cs and cs.vfx and cs.vfx.chromaticAberration
  if Renderer.mode=='full' and chromatic and chromatic.enabled and shaders then
    finalShader=shaders.chromaticAberration
  elseif Renderer.mode=='clean' and shaders then
    -- Clean mode omits the on-top canvas, but its base canvas still contains
    -- palette indices. Apply the same palette/accessibility shader configured
    -- by shuv.finish so it can never regress to the red raw-index output.
    finalShader=(shuv and shuv.usePalette) and shaders.palshader
      or shaders.accessibilityshader
  end
  Renderer.captureSafe(cleanSource, shadedSource, finalShader)
end

return Renderer
