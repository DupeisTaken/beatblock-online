local outbound = love.thread.getChannel('bbt_outbound')
local inbound = love.thread.getChannel('bbt_inbound')
local transport = nil

local function namedPipe()
  local ok, ffi = pcall(require, 'ffi')
  if not ok then return nil end
  ffi.cdef[[
    typedef void* HANDLE; typedef unsigned long DWORD; typedef int BOOL;
    HANDLE CreateFileA(const char*, DWORD, DWORD, void*, DWORD, DWORD, HANDLE);
    BOOL WaitNamedPipeA(const char*, DWORD);
    BOOL PeekNamedPipe(HANDLE, void*, DWORD, DWORD*, DWORD*, DWORD*);
    BOOL ReadFile(HANDLE, void*, DWORD, DWORD*, void*);
    BOOL WriteFile(HANDLE, const void*, DWORD, DWORD*, void*);
    BOOL CloseHandle(HANDLE);
  ]]
  local C = ffi.C
  local name = [[\\.\pipe\beatblock-together-v1]]
  C.WaitNamedPipeA(name, 1000)
  local handle = C.CreateFileA(name, 0xC0000000, 0, nil, 3, 0, nil)
  if handle == ffi.cast('HANDLE', -1) then return nil end
  local buffer = ffi.new('uint8_t[65536]')
  local count = ffi.new('DWORD[1]')
  local available = ffi.new('DWORD[1]')
  return {
    send = function(value)
      local data = value .. '\n'; local written = ffi.new('DWORD[1]')
      return C.WriteFile(handle, data, #data, written, nil) ~= 0
    end,
    receive = function()
      if C.PeekNamedPipe(handle, nil, 0, nil, available, nil) == 0 or available[0] == 0 then return nil end
      local size = math.min(tonumber(available[0]), 65535)
      if C.ReadFile(handle, buffer, size, count, nil) == 0 then return nil end
      return ffi.string(buffer, tonumber(count[0]))
    end,
    close = function() C.CloseHandle(handle) end,
  }
end
local function tcp()
  local ok, socket = pcall(require, 'socket')
  if not ok then return nil end
  local client = socket.tcp(); client:settimeout(0.25)
  if not client:connect('127.0.0.1', 8975) then client:close(); return nil end
  client:settimeout(0)
  return {
    send = function(value) return client:send(value .. '\n') ~= nil end,
    receive = function() local value = client:receive('*l'); return value end,
    close = function() client:close() end,
  }
end

local remainder = ''
while true do
  if not transport then transport = namedPipe() or tcp() end
  if transport then
    while outbound:getCount() > 0 do if not transport.send(outbound:pop()) then transport.close(); transport = nil; break end end
    if transport then
      local chunk = transport.receive()
      if chunk then
        remainder = remainder .. chunk
        while remainder:find('\n', 1, true) do
          local at = remainder:find('\n', 1, true)
          local line = remainder:sub(1, at - 1); remainder = remainder:sub(at + 1)
          if #line > 0 then inbound:push(line) end
        end
      end
    end
  end
  love.timer.sleep(0.005)
end
