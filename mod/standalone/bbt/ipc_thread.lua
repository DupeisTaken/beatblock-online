-- Runs outside the gameplay loop. It owns all blocking IPC and process startup.
local outbound = love.thread.getChannel('bbt_outbound')
local inbound = love.thread.getChannel('bbt_inbound')
local control = love.thread.getChannel('bbt_ipc_control')
local PROTOCOL_VERSION = 3
inbound:push('{"version":'..PROTOCOL_VERSION..',"type":"runtime.launch_status","sequence":0,"runTimeUs":0,"payload":{"phase":"worker source entered"}}')
require('love.timer')
local transport, remainder = nil, ''
local modPath = love.thread.getChannel('bbt_mod_path'):demand()
local launchAttempts, nextLaunchAt = 0, 0
local ffiOk, ffi = pcall(require, 'ffi')

-- Declare the Windows ABI once per worker state. LuaJIT rejects duplicate
-- HANDLE/DWORD typedefs, which previously stopped the worker after its pipe
-- probe and before the hidden runtime launch.
if ffiOk then ffi.cdef[[
  typedef void* HANDLE; typedef uint32_t DWORD; typedef int32_t BOOL;
  typedef struct {
    DWORD cb; char* lpReserved; char* lpDesktop; char* lpTitle;
    DWORD dwX; DWORD dwY; DWORD dwXSize; DWORD dwYSize;
    DWORD dwXCountChars; DWORD dwYCountChars; DWORD dwFillAttribute;
    DWORD dwFlags; uint16_t wShowWindow; uint16_t cbReserved2;
    uint8_t* lpReserved2; HANDLE hStdInput; HANDLE hStdOutput; HANDLE hStdError;
  } BBT_STARTUPINFOA;
  typedef struct { HANDLE hProcess; HANDLE hThread; DWORD dwProcessId; DWORD dwThreadId; } BBT_PROCESS_INFORMATION;
  BOOL CreateProcessA(const char*, char*, void*, void*, BOOL, DWORD, void*, const char*, BBT_STARTUPINFOA*, BBT_PROCESS_INFORMATION*);
  DWORD GetCurrentProcessId(void); BOOL CloseHandle(HANDLE); DWORD GetLastError(void);
  HANDLE CreateFileA(const char*, DWORD, DWORD, void*, DWORD, DWORD, HANDLE);
  BOOL WaitNamedPipeA(const char*, DWORD); BOOL PeekNamedPipe(HANDLE, void*, DWORD, DWORD*, DWORD*, DWORD*);
  BOOL ReadFile(HANDLE, void*, DWORD, DWORD*, void*); BOOL WriteFile(HANDLE, const void*, DWORD, DWORD*, void*);
]] end

local function runtimeError(message)
  inbound:push('{"version":'..PROTOCOL_VERSION..',"type":"runtime.error","sequence":0,"runTimeUs":0,"payload":{"message":' .. string.format('%q', message) .. '}}')
end

local function runtimeStatus(phase)
  inbound:push('{"version":'..PROTOCOL_VERSION..',"type":"runtime.launch_status","sequence":0,"runTimeUs":0,"payload":{"phase":' .. string.format('%q', phase) .. '}}')
end

local function runtimeDisconnected(detail)
  local message=detail or 'The runtime disconnected before the Online action completed. Reconnecting; please retry.'
  inbound:push('{"version":'..PROTOCOL_VERSION..',"type":"runtime.disconnected","sequence":0,"runTimeUs":0,"payload":{"phase":"runtime disconnected; reconnecting","message":'..string.format('%q',message)..'}}')
end

local function launchRuntimeHidden()
  local now = love.timer.getTime()
  if now < nextLaunchAt then return end
  launchAttempts = launchAttempts + 1
  runtimeStatus('launching runtime')
  -- Retry for the lifetime of the Online session, while capping process churn.
  -- A successful IPC connection resets the budget for future runtime crashes.
  nextLaunchAt = now + math.min(8, 2 ^ math.min(launchAttempts - 1, 3))
  if not ffiOk or not modPath then runtimeError('Windows runtime launcher is unavailable.'); return end
  local file = io.open(modPath .. '/runtime-path.txt', 'rb')
  local runtimePath = file and file:read('*a') or nil
  if file then file:close() end
  if not runtimePath or runtimePath == '' then
    runtimeError('Runtime is missing. Open Beatblock Online Installer and choose Repair.')
    return
  end
  runtimePath = runtimePath:gsub('[\r\n]+$', '')
  -- CreateProcess returns after process creation and never waits for the
  -- windowless runtime to reach GUI input-idle.
  local command = '"'..runtimePath..'" --parent-pid '..tostring(ffi.C.GetCurrentProcessId())
  local commandBuffer=ffi.new('char[?]',#command+1)
  ffi.copy(commandBuffer,command)
  local startup=ffi.new('BBT_STARTUPINFOA[1]'); startup[0].cb=ffi.sizeof('BBT_STARTUPINFOA')
  local process=ffi.new('BBT_PROCESS_INFORMATION[1]')
  local result=ffi.C.CreateProcessA(runtimePath,commandBuffer,nil,nil,0,0x08000000,nil,nil,startup,process)
  if result~=0 then
    ffi.C.CloseHandle(process[0].hThread); ffi.C.CloseHandle(process[0].hProcess)
    runtimeStatus('runtime process started')
  else
    runtimeError('Windows could not start the runtime (CreateProcess '..tostring(tonumber(ffi.C.GetLastError()))..').')
  end
end

local pipeSendReported, pipeEmptyReported = false, false
local handshakeMessage = nil
local lastTransportError = nil
local function namedPipe()
  if not ffiOk then return nil end
  local C, name = ffi.C, [[\\.\pipe\beatblock-online-v3]]
  -- Open first: Beatblock's LuaJIT process can receive a false preflight result
  -- from WaitNamedPipeA even while the runtime has an available instance.
  -- Windows only requires waiting after CreateFile reports ERROR_PIPE_BUSY.
  local handle = C.CreateFileA(name, 0xC0000000, 0, nil, 3, 0, nil)
  if tonumber(ffi.cast('intptr_t', handle)) == -1 then
    if tonumber(C.GetLastError()) == 231 then C.WaitNamedPipeA(name, 250) end
    return nil
  end
  local buffer, count, available = ffi.new('uint8_t[65536]'), ffi.new('DWORD[1]'), ffi.new('DWORD[1]')
  return {
    send=function(value)
      if value:find('client.hello', 1, true) then handshakeMessage=value end
      local data=value..'\n'; local bytes=ffi.cast('const uint8_t*',data); local offset=0
      while offset<#data do
        local written=ffi.new('DWORD[1]')
        if C.WriteFile(handle,bytes+offset,#data-offset,written,nil)==0 or written[0]==0 then
          lastTransportError='Named pipe write failed ('..tostring(tonumber(C.GetLastError()))..').'
          return false
        end
        offset=offset+tonumber(written[0])
      end
      if not pipeSendReported then runtimeStatus('sent '..tostring(offset)..' pipe bytes'); pipeSendReported=true end
      return true
    end,
    receive=function()
      if C.PeekNamedPipe(handle,nil,0,nil,available,nil)==0 then
        lastTransportError='Named pipe read failed ('..tostring(tonumber(C.GetLastError()))..').'
        return false
      end
      if available[0]==0 then
        if not pipeEmptyReported then runtimeStatus('pipe connected; awaiting reply'); pipeEmptyReported=true end
        return nil
      end
      local size=math.min(tonumber(available[0]),65535)
      if C.ReadFile(handle,buffer,size,count,nil)==0 then
        lastTransportError='Named pipe read failed ('..tostring(tonumber(C.GetLastError()))..').'
        return false
      end
      return ffi.string(buffer,tonumber(count[0]))
    end,
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
local function disconnectTransport()
  if not transport then return end
  transport.close(); transport=nil; remainder=''
  connectedReported=false; bytesReported=false; pipeSendReported=false
  pipeEmptyReported=false
  nextLaunchAt=love.timer.getTime()+.25
  runtimeDisconnected(lastTransportError)
  lastTransportError=nil
end
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
    -- is not up yet. LuaSocket remains a fallback for non-FFI builds.
    if not transport and ffiOk then launchRuntimeHidden()
    elseif not transport then transport = tcp() end
  end
  if transport then
    if not connectedReported then
      runtimeStatus('runtime transport connected'); connectedReported=true
      launchAttempts=0; nextLaunchAt=0
      if handshakeMessage and not transport.send(handshakeMessage) then disconnectTransport() end
    end
    if transport then
      local sent=0
      while outbound:getCount()>0 and sent<16 do
        if not transport.send(outbound:pop()) then disconnectTransport(); break end
        sent=sent+1
      end
    end
    if transport then
      local chunk = transport.receive()
      if chunk == false then disconnectTransport()
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
