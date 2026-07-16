-- Runs outside the gameplay loop. It owns all blocking IPC and process startup.
local outbound = love.thread.getChannel('bbt_outbound')
local inbound = love.thread.getChannel('bbt_inbound')
local control = love.thread.getChannel('bbt_ipc_control')
inbound:push('{"version":2,"type":"runtime.launch_status","sequence":0,"runTimeUs":0,"payload":{"phase":"worker source entered"}}')
require('love.timer')
local transport, remainder = nil, ''
local modPath = love.thread.getChannel('bbt_mod_path'):demand()
local launchCount, nextLaunchAt = 0, 0
local ffiOk, ffi = pcall(require, 'ffi')

-- Declare the Windows ABI once per worker state. LuaJIT rejects duplicate
-- HANDLE/DWORD typedefs, which previously stopped the worker after its pipe
-- probe and before the hidden runtime launch.
if ffiOk then ffi.cdef[[
  typedef void* HANDLE; typedef unsigned long DWORD; typedef int BOOL; typedef unsigned short WCHAR;
  unsigned int WinExec(const char*, unsigned int);
  DWORD GetCurrentProcessId(void); BOOL CloseHandle(HANDLE); DWORD GetLastError(void);
  HANDLE CreateFileA(const char*, DWORD, DWORD, void*, DWORD, DWORD, HANDLE);
  BOOL WaitNamedPipeA(const char*, DWORD); BOOL PeekNamedPipe(HANDLE, void*, DWORD, DWORD*, DWORD*, DWORD*);
  BOOL ReadFile(HANDLE, void*, DWORD, DWORD*, void*); BOOL WriteFile(HANDLE, const void*, DWORD, DWORD*, void*);
]] end

local function runtimeError(message)
  inbound:push('{"version":2,"type":"runtime.error","sequence":0,"runTimeUs":0,"payload":{"message":' .. string.format('%q', message) .. '}}')
end

local function runtimeStatus(phase)
  inbound:push('{"version":2,"type":"runtime.launch_status","sequence":0,"runTimeUs":0,"payload":{"phase":' .. string.format('%q', phase) .. '}}')
end

local function launchRuntimeHidden()
  if launchCount >= 2 or love.timer.getTime() < nextLaunchAt then return end
  launchCount = launchCount + 1
  runtimeStatus('launching runtime')
  nextLaunchAt = love.timer.getTime() + (launchCount == 1 and 0.5 or 2.0)
  if not ffiOk or not modPath then runtimeError('Windows runtime launcher is unavailable.'); return end
  local file = io.open(modPath .. '/runtime-path.txt', 'rb')
  local runtimePath = file and file:read('*a') or nil
  if file then file:close() end
  if not runtimePath or runtimePath == '' then
    runtimeError('Runtime is missing. Open Beatblock Together Installer and choose Repair.')
    return
  end
  runtimePath = runtimePath:gsub('[\r\n]+$', '')
  local command = '"' .. runtimePath .. '" --parent-pid ' .. tostring(ffi.C.GetCurrentProcessId())
  local result = tonumber(ffi.C.WinExec(command, 0))
  if result and result > 32 then
    runtimeStatus('runtime process started')
  else
    runtimeError('Windows could not start the runtime (WinExec ' .. tostring(result or 0) .. ').')
  end
end

local pipeSendReported = false
local function namedPipe()
  if not ffiOk then return nil end
  local C, name = ffi.C, [[\\.\pipe\beatblock-together-v2]]
  if C.WaitNamedPipeA(name, 250) == 0 then return nil end
  local handle = C.CreateFileA(name, 0xC0000000, 0, nil, 3, 0, nil)
  if tonumber(ffi.cast('intptr_t', handle)) == -1 then return nil end
  local buffer, count, available = ffi.new('uint8_t[65536]'), ffi.new('DWORD[1]'), ffi.new('DWORD[1]')
  return {
    send=function(value)
      local data=value..'\n'; local bytes=ffi.cast('const uint8_t*',data); local offset=0
      while offset<#data do
        local written=ffi.new('DWORD[1]')
        if C.WriteFile(handle,bytes+offset,#data-offset,written,nil)==0 or written[0]==0 then return false end
        offset=offset+tonumber(written[0])
      end
      if not pipeSendReported then runtimeStatus('sent '..tostring(offset)..' pipe bytes'); pipeSendReported=true end
      return true
    end,
    receive=function() if C.PeekNamedPipe(handle,nil,0,nil,available,nil)==0 then return false end; if available[0]==0 then return nil end; local size=math.min(tonumber(available[0]),65535); if C.ReadFile(handle,buffer,size,count,nil)==0 then return false end; return ffi.string(buffer,tonumber(count[0])) end,
    close=function() C.CloseHandle(handle) end,
  }
end

local function tcp()
  local ok, socket = pcall(require, 'socket')
  if not ok then return nil end
  local client = socket.tcp(); client:settimeout(0.25)
  if not client:connect('127.0.0.1', 8975) then client:close(); return nil end
  client:settimeout(0)
  return { send=function(value) return client:send(value..'\n')~=nil end, receive=function() local value,err=client:receive('*l'); if err=='closed' then return false end; return value end, close=function() client:close() end }
end

local stopping = false
local connectedReported, bytesReported = false, false
runtimeStatus('ipc worker ready')
while not stopping do
  if control:pop() == 'stop' then
    while transport and outbound:getCount() > 0 do transport.send(outbound:pop()) end
    stopping = true
  end
  if stopping then break end
  if not transport then
    transport = namedPipe()
    -- This supported build has LuaJIT FFI, so a missing pipe means the runtime
    -- is not up yet. LuaSocket is isolated to builds where FFI itself is absent;
    -- probing an unavailable C module can block Beatblock's module loader.
    if not transport and ffiOk then launchRuntimeHidden()
    elseif not transport then transport = tcp() end
  end
  if transport then
    if not connectedReported then runtimeStatus('named pipe connected'); connectedReported=true end
    local sent=0
    while outbound:getCount()>0 and sent<16 do
      if not transport.send(outbound:pop()) then transport.close(); transport=nil; break end
      sent=sent+1
    end
    if transport then
      local chunk = transport.receive()
      if chunk == false then transport.close(); transport=nil; nextLaunchAt=love.timer.getTime()+.25
      elseif chunk then
        if not bytesReported then runtimeStatus('runtime bytes received'); bytesReported=true end
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
if transport then transport.close() end
