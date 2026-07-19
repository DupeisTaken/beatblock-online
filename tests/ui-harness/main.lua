-- Deterministic screenshot harness for the real Online Lua modules. The LÖVE
-- runtime and fonts stay in the developer's ignored fixture; this tracked file
-- contains every reproducible state transition and layout assertion.
package.path=package.path..';./?.lua;./?/init.lua'

local fixture=os.getenv('BBT_UI_FIXTURE') or '.'
local output=os.getenv('BBT_UI_OUTPUT') or '.'
local autorun=os.getenv('BBT_UI_AUTORUN')=='1'
love.errorhandler=function(message)
  local file=io.open(output..'/harness-error.txt','wb')
  if file then file:write(debug.traceback(tostring(message),2)); file:close() end
  return function() love.event.quit(1) end
end
project={res={x=600,y=360,cx=300,cy=180}}
local function externalFont(name,size)
  local file=assert(io.open(fixture..'/'..name,'rb'))
  local bytes=file:read('*a'); file:close()
  return love.graphics.newFont(love.filesystem.newFileData(bytes,name),size)
end
fonts={
  main=externalFont('DigitalDisco.ttf',14),
  digitalDisco=externalFont('DigitalDisco-Thin.ttf',12),
}
love.graphics.setFont(fonts.digitalDisco)
mouse={rx=0,ry=0,pressed=nil,disableGameplay=function() end}
em={clearCount=0,clear=function() em.clearCount=em.clearCount+1 end}
shuv={pal={},resetPal=function() end,showBadColors=true}
sounds={}; te=nil
local pendingInputs={}
maininput={pressed=function(_,name) return pendingInputs[name]==true end}

Gamestate={}
function Gamestate:new(name)
  local state={name=name}
  function state:setInit(callback) self.init=callback end
  function state:setUpdate(callback) self.updateState=callback end
  function state:setBgDraw(callback) self.bgDrawState=callback end
  function state:setFgDraw(callback) self.drawState=callback end
  return state
end

local function player(id,name,role,admitted,ready,rank,accuracy,commentator)
  return {
    sessionId=id,displayName=name,role=role,admitted=admitted,connected=true,
    ready=ready,verified=admitted and role~='spectator',rank=rank,accuracy=accuracy,
    validity=admitted and 'valid' or 'pending',commentatorAccess=commentator==true,
  }
end
local chart={
  hash=string.rep('a',64),packageName='signal.zip',songName='Signal Through The Static',
  variant='Expert',expectedMaxHits=842,official=false,transferMode='host_transfer',
}
local participants={
  player('host-1','Host','player',true,true,1,99.82),
  player('request-1','New Challenger With A Very Long Name','player',false,false,nil,nil),
}
for index=2,11 do
  participants[#participants+1]=player('player-'..index,'Player '..index,'player',true,true,index,99.82-index*.07)
end
participants[#participants+1]=player('viewer-1','Room Viewer','spectator',true,false,nil,nil)
participants[#participants+1]=player('caster-1','Caster Desk','spectator',true,false,nil,nil,true)
local roomFixture={
  id='visual-room',name='Saturday Showcase',hostSessionId='host-1',lifecycle='ready',
  admissionMode='host_approval',allowChartTransfers=true,participants=participants,chart=chart,
  forceStart=false,currentSetlistIndex=0,createdAtMs=1,updatedAtMs=1,
  setlist={
    {id='set-1',chart=chart,completed=false},
    {id='set-2',chart={songName='Neon Relay',variant='Hard'},completed=false},
    {id='set-3',chart={songName='Final Circuit',variant='Expert'},completed=false},
  },
}
local baseRenderers={}
for index,id in ipairs({'A','B','C','D'}) do
  baseRenderers[index]={
    id=id,active=index<3,featured=index==1,participantId=index<3 and 'player-'..(index+1) or nil,
    participantName=index<3 and 'Player '..(index+1) or nil,mode='clean',width=1280,height=720,
    fps=60,delayMs=500,healthy=index==1,lastError=nil,
  }
end

BBT={
  version='0.3.0-alpha.2',protocolVersion=3,
  context={sessionId='host-1',playerName='Host',lobbyId='visual-room'},
  lastLobby=roomFixture,companionConnected=true,runtimeStarting=false,connected=true,
  chartVerified=true,hudEnabled=true,settings={hostAddress='192.168.1.24',hostPort=32145,hudEnabled=true},
  renderers=baseRenderers,history={
    {name='Friday Finals',status='CLOSED'},{name='Practice Room',status='SET COMPLETE'},
  },
  diagnostics={protocolVersion=3,runtimeVersion='0.3.0-alpha.2',peerCount=14},
  runtimeSnapshot={joinAddress='203.0.113.24:32145',connection='hosting',chartCacheSizeLabel='384.5 MB / 2 GB'},
}
function BBT.currentPlayer()
  for _,value in ipairs(BBT.lastLobby and BBT.lastLobby.participants or {}) do
    if value.sessionId==BBT.context.sessionId then return value end
  end
end
function BBT.isOrganizer()
  local me=BBT.currentPlayer()
  return me and BBT.lastLobby and me.sessionId==BBT.lastLobby.hostSessionId
end
function BBT.startOnlineRuntime() end
function BBT.command(kind,payload)
  if kind=='broadcast.mirror_set' then BBT.mirrorEnabled=payload.enabled end
end
function BBT.update() end
function BBT.maybeLaunchScheduledChart() return false end
function BBT.exitOnline() end
function BBT.openInstaller() end
function BBT.openOfficialSelect() end
function BBT.openChartSelect() end

local forwardedKey
local forwardedText
local nativeKeyPressed=function(key) forwardedKey=key end
local nativeTextInput=function(text) forwardedText=text end
love.keypressed=nativeKeyPressed
love.textinput=nativeTextInput

local rawSetColor=love.graphics.setColor
local invalidPaletteColors={}
love.graphics.setColor=function(r,g,b,a)
  local palette={
    ['1,0,0']={205,205,205},['0,0,1']={255,52,50},['0,1,0']={224,227,0},
    ['1,1,0']={44,255,57},['1,0,1']={0,222,229},['0,1,1']={63,38,255},
  }
  local key=tostring(r)..','..tostring(g)..','..tostring(b)
  local mapped=palette[key]
  if not mapped and key~='1,1,1' and key~='0,0,0' then invalidPaletteColors[key]=true end
  if a~=nil and a~=1 then invalidPaletteColors[key..',alpha='..tostring(a)]=true end
  if mapped then rawSetColor(mapped[1]/255,mapped[2]/255,mapped[3]/255,a or 1) else rawSetColor(r,g,b,a or 1) end
end

local online=require('bbt.online_state')()
local function reset()
  BBT.lastLobby=roomFixture; BBT.context.sessionId='host-1'; BBT.context.lobbyId='visual-room'
  BBT.companionConnected=true; BBT.runtimeStarting=false; BBT.lastError=nil; BBT.chartTransfer=nil
  BBT.renderers=baseRenderers; BBT.mirrorEnabled=false
  roomFixture.lifecycle='ready'; participants[3].verified=true; participants[3].ready=true
  online.workspace='room'; online.rosterFilter='all'; online.selectedSessionId='host-1'
  online.modal=nil; online.broadcastAdvanced=false; online.focusId='session_primary'
end
local scenarios={
  {'connect',function() reset(); BBT.lastLobby=nil; BBT.context.lobbyId='offline' end},
  {'runtime-failure',function() reset(); BBT.lastLobby=nil; BBT.companionConnected=false; BBT.lastError='Runtime did not answer. Repair the installation or open logs for the complete diagnostic details.' end},
  {'host-form',function() reset(); BBT.lastLobby=nil; online:openForm('host') end},
  {'host-form-validation',function() reset(); BBT.lastLobby=nil; online:openForm('host'); online:submitForm(); assert(online.modal and online.modal.error=='PASSWORD IS REQUIRED') end},
  {'join-form',function() reset(); BBT.lastLobby=nil; online:openForm('join',false) end},
  {'long-error',function() reset(); BBT.lastLobby=nil; BBT.companionConnected=false; BBT.lastError=string.rep('This runtime error is intentionally long and must stay bounded. ',8) end},
  {'host-lobby',function() reset() end},
  {'players-filter',function() reset(); online.rosterFilter='players' end},
  {'pending-admission',function() reset(); online.rosterFilter='pending'; online.selectedSessionId='request-1' end},
  {'participant-inspector',function() reset(); online.selectedSessionId='caster-1' end},
  {'mismatch',function() reset(); BBT.context.sessionId='player-2'; participants[3].verified=false; participants[3].ready=false; online.selectedSessionId='player-2' end},
  {'transfer-offer',function() reset(); BBT.context.sessionId='player-2'; participants[3].verified=false; online.selectedSessionId='player-2'; BBT.chartTransfer={state='offer'} end},
  {'transfer-progress',function() reset(); BBT.context.sessionId='player-2'; participants[3].verified=false; online.selectedSessionId='player-2'; BBT.chartTransfer={state='progress',percent=63} end},
  {'consent-warning',function() reset(); BBT.context.sessionId='player-2'; online.modal={kind='confirm',title='SCRIPT CONTENT',message='This package contains Lua or executable content and requires separate explicit confirmation.',label='ACCEPT',run=function() end} end},
  {'live-results',function() reset(); roomFixture.lifecycle='results'; participants[4].validity='dnf'; participants[5].validity='invalid' end},
  {'setlist',function() reset(); online.workspace='setlist' end},
  {'history',function() reset(); online.workspace='history' end},
  {'settings',function() reset(); online.workspace='settings' end},
  {'help',function() reset(); online.workspace='help' end},
  {'confirmation',function() reset(); online.modal={kind='confirm',title='CLOSE ROOM',message='Close this room and disconnect every participant?',label='CLOSE ROOM',run=function() end} end},
  {'broadcast-basic',function() reset(); online.workspace='broadcast' end},
  {'broadcast-advanced',function() reset(); online.workspace='broadcast'; online.broadcastAdvanced=true end},
  {'broadcast-long-error',function() reset(); online.workspace='broadcast'; BBT.renderers[1].lastError=string.rep('Renderer could not resolve the selected package path. ',6) end},
  {'spectator',function() reset(); BBT.context.sessionId='viewer-1'; online.selectedSessionId='viewer-1' end},
  {'commentator-disabled',function() reset(); BBT.context.sessionId='caster-1'; online.workspace='broadcast'; online.selectedSessionId='player-2' end},
  {'commentator-enabled',function() reset(); BBT.context.sessionId='caster-1'; online.workspace='broadcast'; online.selectedSessionId='player-2'; BBT.mirrorEnabled=true end},
}

local scenarioIndex=0
local frames=0
local capturing=false
local auditFile
local qaCanvas
local function beginNext()
  scenarioIndex=scenarioIndex+1
  if scenarioIndex>#scenarios then
    if auditFile then auditFile:close() end
    love.event.quit(0); return
  end
  invalidPaletteColors={}
  scenarios[scenarioIndex][2](); frames=0; capturing=false
end

function love.load()
  love.window.setTitle('Beatblock Online UI QA')
  love.window.setMode(600,360,{resizable=false,vsync=0})
  if autorun and love.window.minimize then love.window.minimize() end
  love.graphics.getDimensions=function() return 600,360 end
  qaCanvas=love.graphics.newCanvas(600,360)
  love.graphics.setFont(fonts.main)
  online:init()
  assert(em.clearCount==1,'Online must clear retained native menu entities')
  assert(love.graphics.getFont()==fonts.digitalDisco,'Online must refresh the native menu font')
  assert(shuv.showBadColors==false,'Online must enable strict indexed palette rendering')
  online:openForm('host')
  online.modal.values.displayName='Player '..string.char(240,159,142,181)
  love.keypressed('backspace')
  assert(online.modal.values.displayName=='Player ','Backspace must remove one complete UTF-8 character')
  love.keypressed('delete')
  assert(online.modal.values.displayName=='Player','Delete must remove the final character')
  love.keypressed('backspace')
  assert(online.modal.values.displayName=='Playe','Repeated deletion must continue editing the field')
  assert(forwardedKey=='backspace','Online must preserve Beatblock key callbacks while editing')
  online.modal=nil
  love.textinput('native')
  assert(forwardedText=='native','Online must preserve Beatblock text input outside forms')
  online:leave()
  assert(love.keypressed==nativeKeyPressed,'Leaving Online must restore Beatblock key callbacks')
  assert(love.textinput==nativeTextInput,'Leaving Online must restore Beatblock text callbacks')
  assert(love.graphics.getFont()==fonts.digitalDisco,'Leaving Online must restore the native menu font')
  assert(shuv.showBadColors==true,'Leaving Online must restore full-color menu rendering')
  online:init({workspace='setlist'})
  assert(online.workspace=='setlist','Chart selection must restore the Setlist workspace')
  assert(online.focusId=='nav_setlist','Setlist return must restore workspace focus')
  online:leave()
  online:init()
  if autorun then
    auditFile=assert(io.open(output..'/layout-audit.txt','wb'))
    beginNext()
  end
end

function love.update(dt)
  local x,y=love.mouse.getPosition(); mouse.rx=x; mouse.ry=y
  online:updateState(dt); pendingInputs={}; mouse.pressed=nil
  if not autorun then return end
  frames=frames+1
  if frames>=3 and not capturing then
    capturing=true
    local name=scenarios[scenarioIndex][1]
    local image=qaCanvas:newImageData()
    local encoded=image:encode('png')
    local file=assert(io.open(output..'/'..name..'.png','wb'))
    file:write(encoded:getString()); file:close()
    local issues=BBT.layoutAudit and BBT.layoutAudit.issues or {}
    for value in pairs(invalidPaletteColors) do issues[#issues+1]='invalid_palette_color:'..value end
    if name=='connect' then
      local hostLabels=0
      local controls={}
      for _,entry in ipairs(BBT.layoutAudit.text or {}) do
        if entry.value=='HOST A ROOM' then hostLabels=hostLabels+1 end
        if entry.y+entry.h>352 then issues[#issues+1]='connect_bottom_safe_area:'..entry.value end
      end
      for _,entry in ipairs(BBT.layoutAudit.controls or {}) do controls[entry.id]=true end
      if hostLabels~=1 then issues[#issues+1]='connect_host_action_count:'..tostring(hostLabels) end
      for _,id in ipairs({'session_primary','connect_join','connect_spectate','connect_exit'}) do
        if not controls[id] then issues[#issues+1]='connect_missing_control:'..id end
      end
      if controls.connect_host then issues[#issues+1]='connect_duplicate_host_action' end
    end
    auditFile:write(name..':'..tostring(#issues)..'\n')
    for _,issue in ipairs(issues) do auditFile:write('  '..issue..'\n') end
    beginNext()
  end
end
function love.draw()
  love.graphics.setCanvas(qaCanvas); love.graphics.clear(0,0,0,1)
  if online.bgDrawState then online:bgDrawState() end
  online:drawState()
  love.graphics.setCanvas(); love.graphics.setColor(1,1,1,1); love.graphics.draw(qaCanvas,0,0)
end
function love.mousepressed(x,y,button) mouse.rx=x; mouse.ry=y; mouse.pressed=button end
