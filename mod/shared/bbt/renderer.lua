local Renderer = {
  active = os.getenv('BBT_RENDERER_FRAME_PATH') ~= nil,
  mode = os.getenv('BBT_RENDERER_MODE') or 'clean',
  framePath = os.getenv('BBT_RENDERER_FRAME_PATH'),
  width = tonumber(os.getenv('BBT_RENDERER_WIDTH')) or 1280,
  height = tonumber(os.getenv('BBT_RENDERER_HEIGHT')) or 720,
  fps = tonumber(os.getenv('BBT_RENDERER_FPS')) or 60,
  sequence = 0, lastInputSequence = 0, tapMask = 0, previousTapMask = 0,
  readbackPending = false, playing = false, droppedFrames = 0, nextFrameAt = nil,
}
Renderer.statePath = (Renderer.framePath or ''):gsub('%.bbtframe$', '.bbtstate')

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

function Renderer.init()
  if not Renderer.active then return Renderer end
  Renderer.frameSize = Renderer.width * Renderer.height * 4
  Renderer.frameInterval = 1 / math.max(1, Renderer.fps)
  Renderer.frames = mapFile(Renderer.framePath, 64 + Renderer.frameSize * 3)
  Renderer.inputs = mapFile(Renderer.statePath, 32)
  Renderer.output = love.graphics.newCanvas(Renderer.width, Renderer.height, {format='rgba8', readable=true})
  local window = ffi.C.GetActiveWindow()
  if window ~= nil then ffi.C.ShowWindow(window, 0) end
  _G.BBTRenderer = Renderer
  return Renderer
end

function Renderer.start()
  if not Renderer.active then return end
  local chart, variant = os.getenv('BBT_RENDERER_CHART'), os.getenv('BBT_RENDERER_VARIANT')
  if not chart or chart == '' then return end
  local previous = cs
  cs = bs.load('Game')
  if GameManager and GameManager.transferStateData and previous then GameManager:transferStateData(cs, previous) end
  if previous and previous.leave then previous:leave() end
  cs.bbtRenderer = true
  cs:init(chart, variant ~= '' and variant or nil)
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

local function copyFrame(data)
  if not Renderer.frames or not data then Renderer.readbackPending = false; return end
  local pointer = data.getFFIPointer and data:getFFIPointer() or nil
  if not pointer then Renderer.readbackPending = false; return end
  Renderer.sequence = Renderer.sequence + 1
  local index = Renderer.sequence % 3
  ffi.copy(Renderer.frames.pointer + 64 + index * Renderer.frameSize, pointer, math.min(Renderer.frameSize, data:getSize()))
  ffi.cast('uint64_t*', Renderer.frames.pointer + 44)[0] = Renderer.droppedFrames
  ffi.cast('uint64_t*', Renderer.frames.pointer + 28)[0] = Renderer.sequence
  Renderer.readbackPending = false
end

function Renderer.capture(source)
  if not Renderer.active or not Renderer.frames then return end
  local now = love.timer.getTime()
  Renderer.nextFrameAt = Renderer.nextFrameAt or now
  if now + .0005 < Renderer.nextFrameAt then return end
  if now - Renderer.nextFrameAt >= Renderer.frameInterval then
    local missed = math.floor((now - Renderer.nextFrameAt) / Renderer.frameInterval)
    Renderer.droppedFrames = Renderer.droppedFrames + missed
    Renderer.nextFrameAt = Renderer.nextFrameAt + (missed + 1) * Renderer.frameInterval
  else
    Renderer.nextFrameAt = Renderer.nextFrameAt + Renderer.frameInterval
  end
  if Renderer.readbackPending then
    Renderer.droppedFrames = Renderer.droppedFrames + 1
    ffi.cast('uint64_t*', Renderer.frames.pointer + 44)[0] = Renderer.droppedFrames
    return
  end
  Renderer.update()
  Renderer.output:renderTo(function()
    if Renderer.mode == 'clean' then drawClean() else
      love.graphics.clear(0,0,0,1); love.graphics.setColor(1,1,1,1)
      love.graphics.draw(source, 0, 0, 0, Renderer.width/source:getWidth(), Renderer.height/source:getHeight())
    end
  end)
  Renderer.readbackPending = true
  if love.graphics.readbackTextureAsync then
    local success = pcall(love.graphics.readbackTextureAsync, Renderer.output, copyFrame)
    if success then return end
  end
  local success, data = pcall(love.graphics.readbackTexture, Renderer.output)
  if success then copyFrame(data) else Renderer.readbackPending = false end
end

return Renderer
