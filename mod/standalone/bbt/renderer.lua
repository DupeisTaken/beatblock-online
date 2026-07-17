local Renderer = {
  active = os.getenv('BBT_RENDERER_FRAME_PATH') ~= nil,
  mode = os.getenv('BBT_RENDERER_MODE') or 'clean',
  framePath = os.getenv('BBT_RENDERER_FRAME_PATH'),
  width = tonumber(os.getenv('BBT_RENDERER_WIDTH')) or 1280,
  height = tonumber(os.getenv('BBT_RENDERER_HEIGHT')) or 720,
  fps = tonumber(os.getenv('BBT_RENDERER_FPS')) or 60,
  sequence = 0, captureSequence = 0, lastInputSequence = 0, tapMask = 0, previousTapMask = 0,
  readbackPending = {false,false}, readbackRequests = {nil,nil}, readbackTickets = {0,0},
  playing = false, droppedFrames = 0, nextFrameAt = nil,
}
Renderer.statePath = (Renderer.framePath or ''):gsub('%.bbtframe$', '.bbtstate')
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
]] end

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
  unmapFile(Renderer.inputs)
  unmapFile(Renderer.frames)
  Renderer.inputs, Renderer.frames, Renderer.outputs = nil, nil, nil
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
  if Renderer.width ~= math.floor(Renderer.width) or Renderer.height ~= math.floor(Renderer.height)
    or Renderer.width < 320 or Renderer.width > 1920 or Renderer.height < 180 or Renderer.height > 1080 then
    return failInitialization('renderer dimensions are outside the supported range')
  end
  if Renderer.fps ~= 30 and Renderer.fps ~= 60 then
    return failInitialization('renderer FPS must be 30 or 60')
  end
  Renderer.frameSize = Renderer.width * Renderer.height * 4
  Renderer.frameInterval = 1 / math.max(1, Renderer.fps)
  Renderer.frames = mapFile(Renderer.framePath, 64 + 1920 * 1080 * 4 * 3)
  Renderer.inputs = mapFile(Renderer.statePath, 32)
  if not Renderer.frames or not Renderer.inputs then
    return failInitialization('renderer shared-memory files could not be mapped')
  end
  -- Two canvases keep one GPU readback from forcing an avoidable 60 Hz drop.
  local canvasOk, first, second = pcall(function()
    return love.graphics.newCanvas(Renderer.width, Renderer.height, {format='rgba8', readable=true}),
      love.graphics.newCanvas(Renderer.width, Renderer.height, {format='rgba8', readable=true})
  end)
  if not canvasOk or not first or not second then
    return failInitialization('renderer canvases could not be allocated')
  end
  Renderer.outputs = {first, second}
  local window = ffi.C.GetActiveWindow()
  if window ~= nil then ffi.C.ShowWindow(window, 0) end
  _G.BBTRenderer = Renderer
  return Renderer
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
  -- Gameplay's Play Song event expects preloadSoundData to be a table even
  -- when it has no matching preloaded track. Zero the disposable child's audio
  -- settings before that event takes the normal synchronous-load fallback.
  if savedata and savedata.options and savedata.options.audio then
    for key, value in pairs(savedata.options.audio) do
      if type(value) == 'number' then savedata.options.audio[key] = 0 end
    end
  end
  -- Beatblock's background loader reads the global cLevel even though Game:init
  -- also receives the chart path. Mirror the native Freeplay launch contract.
  cLevel = chart
  cs = bs.load('Game')
  if GameManager and GameManager.transferStateData and previous then GameManager:transferStateData(cs, previous) end
  if previous and previous.leave then previous:leave() end
  cs.bbtRenderer = true
  -- Freeplay normally supplies a table with `path` and `data`. An empty table
  -- preserves that shape and lets Beatblock load the song without attempting
  -- to index a boolean sentinel.
  cs:init(chart, variantInfo, nil, {})
  if cs.source and cs.source.setVolume then cs.source:setVolume(0) end
end

function Renderer.update()
  if not Renderer.active or not Renderer.inputs then return end
  local sequence = tonumber(ffi.cast('uint32_t*', Renderer.inputs.pointer + 8)[0])
  if sequence == Renderer.lastInputSequence then return end
  Renderer.lastInputSequence = sequence
  Renderer.previousTapMask = Renderer.tapMask
  Renderer.tapMask = tonumber(ffi.cast('uint16_t*', Renderer.inputs.pointer + 28)[0])
  Renderer.beat = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 20)[0])
  Renderer.angle = tonumber(ffi.cast('float*', Renderer.inputs.pointer + 24)[0])
  local flags = tonumber(ffi.cast('uint16_t*', Renderer.inputs.pointer + 30)[0])
  Renderer.playing = flags % 2 == 1
  if cs then cs.cBeat = Renderer.beat end
  if cs and cs.p then cs.p.angle = Renderer.angle; cs.p.anglePrevFrame = Renderer.angle end
end

function Renderer.shouldHold()
  return Renderer.active and not Renderer.playing
end

function Renderer.tapInputs()
  return Renderer.tapMask ~= 0 and Renderer.previousTapMask == 0,
    Renderer.tapMask == 0 and Renderer.previousTapMask ~= 0
end

local function drawClean()
  love.graphics.clear(0.025, 0.04, 0.075, 1)
  love.graphics.push()
  love.graphics.scale(Renderer.width / project.res.x, Renderer.height / project.res.y)
  love.graphics.setColor(.2, .3, .45, 1)
  love.graphics.setLineWidth(3)
  local radius = math.min(project.res.x, project.res.y) * .28
  love.graphics.circle('line', project.res.cx, project.res.cy, radius)
  local groups = {{cs and cs.notes, {.35,.72,1,1}}, {cs and cs.taps, {.42,1,.7,1}}, {cs and cs.mines, {1,.35,.4,1}}}
  for _, group in ipairs(groups) do
    love.graphics.setColor(group[2])
    for _, note in ipairs(group[1] or {}) do
      if note.x and note.y and not note.delete then love.graphics.circle('fill', note.x, note.y, note.tap and 7 or 5) end
    end
  end
  local angle = math.rad((Renderer.angle or 0) - 90)
  local px, py = project.res.cx + math.cos(angle) * radius, project.res.cy + math.sin(angle) * radius
  love.graphics.setColor(1,1,1,1); love.graphics.setLineWidth(8)
  love.graphics.line(px - math.cos(angle)*14, py - math.sin(angle)*14, px + math.cos(angle)*14, py + math.sin(angle)*14)
  love.graphics.pop()
end

local function copyFrame(data, readbackSlot, ticket)
  if Renderer.readbackTickets[readbackSlot] ~= ticket then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    return
  end
  Renderer.readbackPending[readbackSlot] = false
  Renderer.readbackRequests[readbackSlot] = nil
  if not Renderer.frames or not data then return end
  local pointer = data.getFFIPointer and data:getFFIPointer() or nil
  if not pointer then return end
  -- Async callbacks are permitted to complete out of order. Never publish an
  -- older capture over a newer one already visible to OBS.
  if ticket <= Renderer.sequence then Renderer.droppedFrames = Renderer.droppedFrames + 1; return end
  Renderer.sequence = ticket
  local index = Renderer.sequence % 3
  ffi.copy(Renderer.frames.pointer + 64 + index * Renderer.frameSize, pointer, math.min(Renderer.frameSize, data:getSize()))
  ffi.cast('uint64_t*', Renderer.frames.pointer + 48)[0] = Renderer.droppedFrames
  -- The aligned sequence is the commit marker and is written after all pixels.
  ffi.cast('uint64_t*', Renderer.frames.pointer + 32)[0] = Renderer.sequence
end

local function finishReadbacks()
  for slot = 1, 2 do
    local request = Renderer.readbackRequests[slot]
    if request then
      local updated = pcall(request.update, request)
      local checked, complete = pcall(request.isComplete, request)
      if not updated or not checked then
        Renderer.readbackRequests[slot] = nil
        Renderer.readbackPending[slot] = false
        Renderer.droppedFrames = Renderer.droppedFrames + 1
      elseif complete then
        local failed = request:hasError()
        local data = not failed and request:getImageData() or nil
        if failed or not data then
          Renderer.readbackRequests[slot] = nil
          Renderer.readbackPending[slot] = false
          Renderer.droppedFrames = Renderer.droppedFrames + 1
        else
          copyFrame(data, slot, Renderer.readbackTickets[slot])
        end
      end
    end
  end
end

function Renderer.capture(source)
  if not Renderer.active or not Renderer.frames then return end
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
  Renderer.update()
  local output=Renderer.outputs[readbackSlot]
  output:renderTo(function()
    if Renderer.mode == 'clean' then drawClean() else
      love.graphics.clear(0,0,0,1); love.graphics.setColor(1,1,1,1)
      love.graphics.draw(source, 0, 0, 0, Renderer.width/source:getWidth(), Renderer.height/source:getHeight())
    end
  end)
  Renderer.captureSequence = Renderer.captureSequence + 1
  local ticket=Renderer.captureSequence
  Renderer.readbackPending[readbackSlot] = true
  Renderer.readbackTickets[readbackSlot] = ticket
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
  end
end

function Renderer.captureSafe(source)
  if Renderer.captureError then return end
  local success, message = xpcall(function() Renderer.capture(source) end, debug.traceback)
  if success then return end
  reportError('renderer capture failed:\n' .. tostring(message))
end

return Renderer
