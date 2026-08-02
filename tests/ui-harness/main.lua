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
  -- Return an exit status immediately. A graphical error loop can remain
  -- alive while minimized, hiding the assertion until the outer timeout.
  return function() return 1 end
end
project={res={x=600,y=360,cx=300,cy=180}}
local function externalFont(name,size)
  local file=assert(io.open(fixture..'/'..name,'rb'))
  local bytes=file:read('*a'); file:close()
  return love.graphics.newFont(love.filesystem.newFileData(bytes,name),size)
end
-- These MUST mirror the shipped game's preload/fonts.lua exactly. Beatblock
-- builds them as:
--   main         = love.graphics.newFont("assets/fonts/Axmolotl.ttf", 16)
--   digitalDisco = love.graphics.newFont("assets/fonts/DigitalDisco-Thin.ttf", 16)
-- Online reasserts fonts.digitalDisco every frame, so any drift between this
-- table and the game means the whole dashboard is baselined at metrics that do
-- not exist in the product. A smaller size here silently hides real text
-- overflow; do not "fix" a layout failure by changing a face or a size.
fonts={
  main=externalFont('Axmolotl.ttf',16),
  digitalDisco=externalFont('DigitalDisco-Thin.ttf',16),
}
love.graphics.setFont(fonts.digitalDisco)
mouse={rx=0,ry=0,pressed=nil,disableGameplay=function() end}
em={clearCount=0,playerInstance={class={name='Player'}}}
function em.clear()
  em.clearCount=em.clearCount+1
  em.playerInstance=nil
end
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
local function setlistEntry(index,name,variant,completed,official)
  return {
    id='set-'..tostring(index),
    chart={
      hash=string.rep(string.char(96+(index%6)+1),64),
      packageName='chart-'..tostring(index)..'.zip',
      songName=name or ('Chart '..tostring(index)),
      variant=variant or (index%2==0 and 'Hard' or 'Expert'),
      expectedMaxHits=800+index,
      official=official==true,
      transferMode=official and 'verify_only' or 'host_transfer',
    },
    completed=completed==true,
  }
end
local function setlistOf(count)
  local entries={}
  for index=1,count do
    entries[index]=setlistEntry(index,index==1 and chart.songName or nil)
  end
  entries[1].chart=chart
  return entries
end
local participants={
  player('host-1','Host','host',true,true,1,99.82),
  player('request-1','New Challenger With A Very Long Name','player',false,false,nil,nil),
}
for index=2,11 do
  participants[#participants+1]=player('player-'..index,'Player '..index,'player',true,true,index,99.82-index*.07)
end
participants[#participants+1]=player('viewer-1','Room Viewer','spectator',true,false,nil,nil)
participants[#participants+1]=player('caster-1','Caster Desk','spectator',true,false,nil,nil,true)
local roomFixture={
  id='visual-room',name='Saturday Showcase',hostSessionId='host-1',lifecycle='ready',
  admissionMode='host_approval',allowChartTransfers=true,validityChecksEnabled=true,requireSameGameBuild=true,participants=participants,chart=chart,
  modifiers={rate=1.0,vfx='full',taps='default',sides='default',barelies='default',restartOn='none'},
  forceStart=false,currentSetlistIndex=0,createdAtMs=1,updatedAtMs=1,
  setlist=setlistOf(3),
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
  version='0.3.1',protocolVersion=3,
  testedBeatblockVersion='1.7.1a',
  context={sessionId='host-1',playerName='Host',lobbyId='visual-room'},
  lastLobby=roomFixture,companionConnected=true,runtimeStarting=false,connected=true,
  chartVerified=true,hudEnabled=true,settings={
    hostAddress='192.168.1.24',hostPort=32145,hudEnabled=true,rendererDesktopMute=true,
  },
  renderers=baseRenderers,history={
    {name='Friday Finals',status='CLOSED'},{name='Practice Room',status='SET COMPLETE'},
  },
  diagnostics={
    protocolVersion=3,runtimeVersion='0.3.1',peerCount=14,
    testedBeatblockVersion='1.7.1a',
    testedBeatblockBuildId='d40b7083',
    detectedBeatblockVersion='1.7.1a (Early Access)[d40b7083]',
    detectedBeatblockBuildId='d40b7083',
    detectedBeatblockBuildSource='displayed_build_hash',
  },
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
function BBT.restoreRoomModifiers() end
BBT.commandLog={}
function BBT.command(kind,payload)
  BBT.commandLog[#BBT.commandLog+1]={kind=kind,payload=payload}
  if kind=='broadcast.mirror_set' then BBT.mirrorEnabled=payload.enabled end
  return 'harness-request-'..tostring(#BBT.commandLog)
end
function BBT.update() end
function BBT.maybeLaunchScheduledChart() return false end
function BBT.exitOnline() end
function BBT.openInstaller() end
BBT.selectorLog={}
function BBT.openOfficialSelect(mode)
  BBT.selectorLog[#BBT.selectorLog+1]={source='official',mode=mode}
end
function BBT.openChartSelect(mode)
  BBT.selectorLog[#BBT.selectorLog+1]={source='custom',mode=mode}
end

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

local Dashboard=require('bbt.dashboard_model')
local online=require('bbt.online_state')()
local function reset()
  BBT.lastLobby=roomFixture; BBT.context.sessionId='host-1'; BBT.context.lobbyId='visual-room'
  BBT.companionConnected=true; BBT.runtimeStarting=false; BBT.lastError=nil; BBT.chartTransfer=nil
  BBT.renderers=baseRenderers; BBT.mirrorEnabled=false
  BBT.commandLog={}; BBT.pendingRequestId=nil; BBT.lastCompletedRequestId=nil
  BBT.selectorLog={}; BBT.chartVerified=true
  BBT.settings.hudEnabled=true; BBT.settings.rendererDesktopMute=true
  roomFixture.allowChartTransfers=true; roomFixture.autoRequestChartTransfers=false
  roomFixture.validityChecksEnabled=true; roomFixture.requireSameGameBuild=true
  roomFixture.modifiers={rate=1.0,vfx='full',taps='default',sides='default',barelies='default',restartOn='none'}
  roomFixture.chart=chart; roomFixture.setlist=setlistOf(3); roomFixture.currentSetlistIndex=0
  for _,participant in ipairs(participants) do
    participant.validity=participant.admitted and 'valid' or 'pending'
    participant.invalidReason=nil
  end
  participants[1].role='host'; participants[1].ready=true; participants[1].verified=true
  roomFixture.lifecycle='ready'; participants[3].verified=true; participants[3].ready=true
  online.workspace='room'; online.rosterFilter='all'; online.selectedSessionId='host-1'
  online.modal=nil; online.broadcastAdvanced=false; online.broadcastSlot='A'; online.broadcastDraft=nil
  online.setlistSelection=1; online.selectedSetlistEntryId=nil; online.setlistOffset=0
  online.advanceRequestId=nil; online.advancePreviousHash=nil; online.focusId='session_primary'
end
local scenarios={
  {'connect',function() reset(); BBT.lastLobby=nil; BBT.context.lobbyId='offline' end},
  {'runtime-failure',function() reset(); BBT.lastLobby=nil; BBT.companionConnected=false; BBT.lastError='Runtime did not answer. Repair the installation or open logs for the complete diagnostic details.' end},
  {'host-form',function() reset(); BBT.lastLobby=nil; online:openForm('host') end},
  {'host-form-directing',function() reset(); BBT.lastLobby=nil; online:openForm('host'); online.modal.values.hostParticipating=false end},
  {'host-form-checks-off',function() reset(); BBT.lastLobby=nil; online:openForm('host'); online.modal.values.validityChecksEnabled=false end},
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
  {'live-results',function() reset(); roomFixture.lifecycle='results'; participants[4].validity='dnf'; participants[5].validity='invalid'; participants[5].invalidReason='Missing ordered score event 27'; online.selectedSessionId='player-4' end},
  {'host-directing',function() reset(); participants[1].role='spectator'; participants[1].ready=true; participants[1].verified=true; online.selectedSessionId='host-1' end},
  {'setlist-empty',function()
    reset(); roomFixture.chart=nil; roomFixture.setlist={}; roomFixture.currentSetlistIndex=nil
    roomFixture.lifecycle='forming'; online.workspace='setlist'
  end},
  {'setlist',function() reset(); online.workspace='setlist' end},
  {'setlist-six',function()
    reset(); roomFixture.setlist=setlistOf(6); online.workspace='setlist'
  end},
  {'setlist-overflow',function()
    reset(); roomFixture.setlist=setlistOf(9); online.workspace='setlist'
    online.selectedSetlistEntryId='set-8'; online.setlistSelection=8
  end},
  {'setlist-results',function()
    reset(); roomFixture.lifecycle='results'; roomFixture.setlist[1].completed=true
    online.workspace='setlist'; online.selectedSetlistEntryId='set-2'; online.setlistSelection=2
  end},
  {'setlist-complete',function()
    reset(); roomFixture.lifecycle='set_complete'; roomFixture.currentSetlistIndex=2
    for _,entry in ipairs(roomFixture.setlist) do entry.completed=true end
    online.workspace='setlist'; online.selectedSetlistEntryId='set-3'; online.setlistSelection=3
  end},
  {'setlist-locked',function()
    reset(); roomFixture.lifecycle='playing'; roomFixture.setlist=setlistOf(6)
    online.workspace='setlist'; online.selectedSetlistEntryId='set-3'; online.setlistSelection=3
  end},
  {'setlist-readonly',function()
    reset(); roomFixture.setlist=setlistOf(6); BBT.context.sessionId='player-2'
    online.workspace='setlist'; online.selectedSetlistEntryId='set-3'; online.setlistSelection=3
  end},
  {'setlist-unicode',function()
    reset(); roomFixture.setlist=setlistOf(6)
    roomFixture.setlist[3].chart.songName=string.rep(string.char(231,149,140),20)..' Finale'
    roomFixture.setlist[3].chart.variant='超長難度名'
    online.workspace='setlist'; online.selectedSetlistEntryId='set-3'; online.setlistSelection=3
  end},
  {'single-chart-source',function()
    reset(); roomFixture.chart=nil; roomFixture.setlist={}; roomFixture.currentSetlistIndex=nil
    roomFixture.lifecycle='forming'; online:openSingleChartSource()
  end},
  {'local-chart-source',function()
    reset(); BBT.context.sessionId='player-2'; participants[3].verified=false
    participants[3].ready=false; online.selectedSessionId='player-2'
    online:openSingleChartSource('verify')
  end},
  {'single-chart-replace',function()
    reset(); roomFixture.lifecycle='set_complete'; roomFixture.currentSetlistIndex=2
    for _,entry in ipairs(roomFixture.setlist) do entry.completed=true end
    online:openSingleChartSource(); online:chooseSingleChartSource(true)
  end},
  {'history',function() reset(); online.workspace='history' end},
  {'settings',function() reset(); online.workspace='settings' end},
  {'settings-modifiers',function()
    reset(); online.workspace='settings'; online:openModifiers()
    online.modal.values={rate=1.7,vfx='decreased',taps='strict',sides='lenient',barelies='strict',restartOn='miss'}
  end},
  {'settings-modifiers-readonly',function()
    reset(); online.workspace='settings'; BBT.context.sessionId='player-2'; online:openModifiers()
    online.modal.values={rate=1.7,vfx='decreased',taps='strict',sides='lenient',barelies='strict',restartOn='miss'}
  end},
  {'settings-casual',function() reset(); online.workspace='settings'; roomFixture.validityChecksEnabled=false end},
  {'settings-automatic',function()
    reset(); online.workspace='settings'; roomFixture.autoRequestChartTransfers=true
  end},
  {'settings-maximum',function()
    reset(); online.workspace='settings'; roomFixture.validityChecksEnabled=false
    roomFixture.requireSameGameBuild=false; roomFixture.autoRequestChartTransfers=true
    BBT.settings.hudEnabled=false; BBT.settings.rendererDesktopMute=false
  end},
  {'settings-locked',function()
    reset(); online.workspace='settings'; roomFixture.lifecycle='playing'
  end},
  {'settings-readonly',function()
    reset(); online.workspace='settings'; BBT.context.sessionId='player-2'
  end},
  {'help',function() reset(); online.workspace='help' end},
  {'confirmation',function() reset(); online.modal={kind='confirm',title='CLOSE ROOM',message='Close this room and disconnect every participant?',label='CLOSE ROOM',run=function() end} end},
  {'broadcast-basic',function() reset(); online.workspace='broadcast' end},
  {'broadcast-advanced',function()
    reset(); online.workspace='broadcast'; online.broadcastAdvanced=true; online.broadcastSlot='B'
    online.broadcastDraft={mode='clean',width=1920,height=1080,fps=60,delayMs=1000}
  end},
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
  -- CI and developer machines may run at 100-150% display scaling. The QA
  -- artifact is the fixed logical game canvas, never a monitor-scaled texture.
  love.window.setMode(600,360,{resizable=false,vsync=0,highdpi=false})
  if autorun and love.window.minimize then love.window.minimize() end
  love.graphics.getDimensions=function() return 600,360 end
  qaCanvas=love.graphics.newCanvas(600,360,{dpiscale=1})
  love.graphics.setFont(fonts.main)
  online:init()
  assert(em.clearCount==1,'Online must clear retained native menu entities')
  assert(em.playerInstance==nil,'Online must clear the inherited native Player instance')
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
  em.playerInstance={class={name='Player'}}
  online:leave()
  assert(em.clearCount==2 and em.playerInstance==nil,
    'Leaving Online must clear the retained native Player instance')
  assert(love.keypressed==nativeKeyPressed,'Leaving Online must restore Beatblock key callbacks')
  assert(love.textinput==nativeTextInput,'Leaving Online must restore Beatblock text callbacks')
  assert(love.graphics.getFont()==fonts.digitalDisco,'Leaving Online must restore the native menu font')
  assert(shuv.showBadColors==true,'Leaving Online must restore full-color menu rendering')
  online:init({workspace='setlist'})
  assert(online.workspace=='setlist','Chart selection must restore the Setlist workspace')
  assert(online.focusId=='nav_setlist','Setlist return must restore workspace focus')

  local function optionalControl(id)
    online:drawState()
    for _,control in ipairs(online.controls or {}) do
      if control.id==id then return control end
    end
  end
  local function findControl(id)
    local control=optionalControl(id)
    if control then return control end
    error('Missing UI control '..id)
  end
  local function activate(id)
    local control=findControl(id)
    assert(control.run,'Control '..id..' is unexpectedly disabled')
    control.run()
  end

  reset()
  activate('participant_host_play')
  assert(BBT.commandLog[1].kind=='room.host_play_set','Host play toggle must use its dedicated runtime command')
  assert(BBT.commandLog[1].payload.participating==false,'Playing host toggle must switch to directing')

  reset(); BBT.lastLobby=nil; online:openForm('host')
  assert(online.modal.values.hostParticipating==true,'Host room creation must default to playing for compatibility')
  assert(online.modal.values.requireSameGameBuild==true,'Host room creation must require the exact Beatblock build by default')
  activate('form_host_direct')
  activate('form_checks_off')
  activate('form_build_any')
  online.modal.values.password='secret'
  activate('form_submit')
  assert(BBT.commandLog[1].kind=='room.host_request','Host form must submit a room creation request')
  assert(BBT.commandLog[1].payload.hostParticipating==false,'Director choice must cross the game/runtime bridge')
  assert(BBT.commandLog[1].payload.validityChecksEnabled==false,'Run-check choice must cross the game/runtime bridge')
  assert(BBT.commandLog[1].payload.requireSameGameBuild==false,'Same-build choice must cross the game/runtime bridge')

  reset(); BBT.lastLobby=nil; online:openForm('host'); online.modal.values.password='secret'
  local commandBeforeFailure=BBT.command
  BBT.command=function() BBT.lastError='Runtime is busy'; return nil end
  activate('form_submit')
  assert(online.modal and online.modal.kind=='form','A rejected host command must keep the form open')
  assert(online.modal.error=='Runtime is busy','A rejected host command must explain why it stayed open')
  BBT.command=commandBeforeFailure

  reset(); online.workspace='settings'
  online:drawState()
  local settingsText={}
  for _,entry in ipairs(BBT.layoutAudit.text or {}) do settingsText[entry.value]=true end
  assert(settingsText['BEATBLOCK ONLINE'],'Header must identify the product without a duplicate abbreviation')
  assert(settingsText['v0.3.1  /  READY'],'Header must show version and concise runtime state')
  assert(settingsText['v0.3.1'],'Compatibility must show the installed Online version')
  assert(settingsText['V3 / MATCH'],'Compatibility must document the matching protocol')
  assert(settingsText['1.7.1a+'],'Compatibility must identify the tested Beatblock baseline')
  assert(settingsText['BUILD [d40b7083]'],'Compatibility must show the running game build token')
  for _,label in ipairs({
    'HUD: ON','RUN CHECKS: ON','BUILD: SAME','REQUESTS: MANUAL',
    'MODIFIERS: DEFAULT','DESKTOP MUTE: ON','CLEAR TRANSFER CACHE',
  }) do
    assert(settingsText[label],'Settings must expose the complete state/action label '..label)
  end
  activate('settings_hud')
  assert(BBT.commandLog[1].kind=='settings.update' and BBT.commandLog[1].payload.hudEnabled==false,
    'Settings HUD state control must emit the disabled state')

  reset(); online.workspace='settings'
  activate('settings_renderer_mute')
  assert(BBT.commandLog[1].kind=='settings.update'
    and BBT.commandLog[1].payload.rendererDesktopMute==false,
    'Desktop mute state control must preserve the renderer isolation setting')

  reset(); online.workspace='settings'
  activate('settings_transfer_policy')
  assert(BBT.commandLog[1].kind=='room.chart_transfer_policy_set'
    and BBT.commandLog[1].payload.autoRequest==true,
    'Settings Requests state control must enable automatic requests explicitly')

  reset(); online.workspace='settings'
  activate('settings_modifiers')
  assert(online.modal and online.modal.kind=='modifiers','Room modifiers must open a dedicated policy editor')
  activate('modifier_rate_up')
  activate('modifier_taps_strict')
  activate('modifier_sides_lenient')
  activate('modifier_barelies_strict')
  activate('modifier_restartOn_miss')
  activate('modifiers_apply')
  assert(BBT.commandLog[1].kind=='room.modifiers_set','Modifier editor must use the dedicated room command')
  assert(BBT.commandLog[1].payload.modifiers.rate==1.1
    and BBT.commandLog[1].payload.modifiers.taps=='strict'
    and BBT.commandLog[1].payload.modifiers.restartOn=='miss',
    'Modifier editor must submit the complete native policy')

  reset(); online.workspace='settings'
  activate('settings_clear_cache')
  assert(online.modal and online.modal.kind=='confirm','Clearing transfer cache must remain destructive')
  activate('modal_confirm')
  assert(BBT.commandLog[1].kind=='chart.cache_clear','Transfer cache confirmation must emit the cache command')

  reset(); online.workspace='settings'
  activate('settings_validity')
  assert(online.modal and online.modal.kind=='confirm','Disabling run checks must explain the competitive tradeoff')
  activate('modal_confirm')
  assert(BBT.commandLog[1].kind=='room.validity_checks_set','Settings must use the dedicated run-check command')
  assert(BBT.commandLog[1].payload.enabled==false,'Settings must disable checks explicitly')

  reset(); online.workspace='settings'
  activate('settings_build_policy')
  assert(online.modal and online.modal.kind=='confirm','Allowing mixed Beatblock builds must explain the integrity tradeoff')
  activate('modal_confirm')
  assert(BBT.commandLog[1].kind=='room.game_build_policy_set','Settings must use the dedicated build-policy command')
  assert(BBT.commandLog[1].payload.required==false,'Settings must relax build matching explicitly')

  reset(); online.workspace='settings'; roomFixture.validityChecksEnabled=false
  activate('settings_validity')
  assert(BBT.commandLog[1].kind=='room.validity_checks_set' and BBT.commandLog[1].payload.enabled==true,'Settings must re-enable checks without a destructive confirmation')

  reset(); online.workspace='settings'; roomFixture.requireSameGameBuild=false
  activate('settings_build_policy')
  assert(BBT.commandLog[1].kind=='room.game_build_policy_set'
    and BBT.commandLog[1].payload.required==true,
    'Settings Build state control must restore exact matching without a destructive confirmation')

  reset(); online.workspace='settings'; roomFixture.lifecycle='playing'; online:drawState()
  for _,id in ipairs({'settings_validity','settings_build_policy','settings_transfer_policy'}) do
    assert(not optionalControl(id),'Host room policy must lock during play: '..id)
  end
  for _,id in ipairs({'settings_hud','settings_modifiers','settings_renderer_mute','settings_clear_cache'}) do
    assert(optionalControl(id),'Local Settings action must remain available during play: '..id)
  end

  reset(); online.workspace='settings'; BBT.context.sessionId='player-2'; online:drawState()
  for _,id in ipairs({'settings_validity','settings_build_policy','settings_transfer_policy'}) do
    assert(not optionalControl(id),'Non-host room policy must remain read-only: '..id)
  end
  activate('settings_modifiers')
  assert(online.modal and online.modal.kind=='modifiers' and not online.modal.editable,
    'Non-hosts must be able to inspect but not edit the host modifier policy')
  assert(not optionalControl('modifiers_apply'),'Read-only modifier policy must not expose Apply')

  reset(); online.workspace='settings'; roomFixture.validityChecksEnabled=false
  roomFixture.requireSameGameBuild=false; roomFixture.autoRequestChartTransfers=true
  BBT.settings.hudEnabled=false; BBT.settings.rendererDesktopMute=false
  online:drawState()
  local maximumSettings={}
  for _,entry in ipairs(BBT.layoutAudit.text or {}) do maximumSettings[entry.value]=true end
  for _,label in ipairs({
    'HUD: OFF','RUN CHECKS: OFF','BUILD: ANY','REQUESTS: AUTO','DESKTOP MUTE: OFF',
  }) do
    assert(maximumSettings[label],'Maximum Settings combination must preserve '..label)
  end

  reset(); roomFixture.lifecycle='results'; participants[5].validity='invalid'; participants[5].invalidReason='Missing ordered score event 27'; online.selectedSessionId='player-4'
  activate('participant_run_details')
  assert(online.modal and online.modal.message=='Missing ordered score event 27','Invalid result details must expose the authoritative reason')

  reset(); roomFixture.lifecycle='results'
  -- Select from the sorted roster, not the insertion-order fixture: pending
  -- requests and host-first ordering deliberately move entries between pages.
  local orderedParticipants=Dashboard.visibleParticipants({room=roomFixture},'all')
  local offPageParticipant=orderedParticipants[#orderedParticipants]
  offPageParticipant.validity='invalid'; offPageParticipant.invalidReason=nil
  online.selectedSessionId=offPageParticipant.sessionId
  activate('participant_run_details')
  assert(online.rosterOffset>0,'Selecting an off-page invalid result must reveal its roster page')
  assert(online.modal and online.modal.message:find('did not provide',1,true),'Invalid results without a legacy reason must still expose details')

  reset(); roomFixture.setlist=setlistOf(4); online.workspace='setlist'
  online.selectedSetlistEntryId='set-3'; online.setlistSelection=3
  online:drawState()
  local setlistUp=findControl('setlist_up')
  local setlistDown=findControl('setlist_down')
  local setlistRemove=findControl('setlist_remove')
  assert(setlistUp.x+setlistUp.w<=setlistDown.x
    and setlistDown.x+setlistDown.w<=setlistRemove.x,
    'Setlist row actions must remain in Move Up, Move Down, Remove visual order')
  assert(online.selectedSetlistEntryId=='set-3' and online.setlistSelection==3,
    'Setlist selection must resolve the selected entry id')
  roomFixture.setlist[3],roomFixture.setlist[4]=roomFixture.setlist[4],roomFixture.setlist[3]
  online:drawState()
  assert(online.selectedSetlistEntryId=='set-3' and online.setlistSelection==4,
    'Setlist selection must follow a stable entry id across async snapshot reorder')
  activate('setlist_up')
  assert(BBT.commandLog[1].kind=='setlist.move','Setlist Up must emit a move command')
  assert(BBT.commandLog[1].payload.from==3 and BBT.commandLog[1].payload.to==2,
    'Setlist ordering must resolve the current stable id to zero-based runtime indexes')
  assert(online.selectedSetlistEntryId=='set-3',
    'A pending move must retain selected identity until the authoritative snapshot arrives')

  local function assertActiveSetlistBoundary(lifecycle)
    reset(); roomFixture.lifecycle=lifecycle; online.workspace='setlist'
    online.selectedSetlistEntryId='set-1'; online.setlistSelection=1; online:drawState()
    assert(not optionalControl('setlist_down'),
      lifecycle..' must not move the active chart into the future queue')
    online.selectedSetlistEntryId='set-2'; online.setlistSelection=2; online:drawState()
    assert(not optionalControl('setlist_up'),
      lifecycle..' must not move a future chart across the active boundary')
    assert(optionalControl('setlist_down'),
      lifecycle..' must still allow reordering entirely within the future queue')
  end
  assertActiveSetlistBoundary('chart_locked')
  assertActiveSetlistBoundary('ready')

  reset(); online.workspace='setlist'; online.selectedSetlistEntryId='set-2'; online.setlistSelection=2
  online:drawState(); table.remove(roomFixture.setlist,2); online:drawState()
  assert(online.selectedSetlistEntryId=='set-3' and online.setlistSelection==2,
    'A removed selected entry must fall back to the adjacent authoritative row')

  reset(); roomFixture.chart=nil; roomFixture.setlist={}; roomFixture.currentSetlistIndex=nil
  roomFixture.lifecycle='forming'
  activate('session_primary')
  assert(online.modal and online.modal.kind=='chart_source',
    'Global Select Chart must open the single-chart source dialog')
  activate('single_chart_official')
  assert(#BBT.selectorLog==1 and BBT.selectorLog[1].source=='official'
    and BBT.selectorLog[1].mode=='single',
    'One-off official selection must leave for Beatblock only after the source choice')

  reset(); BBT.context.sessionId='player-2'; participants[3].verified=false
  participants[3].ready=false; online.selectedSessionId='player-2'
  activate('session_local_chart')
  assert(online.modal and online.modal.kind=='chart_source'
    and online.modal.selectionMode=='verify',
    'Top Find Local must open the shared chart source dialog in verification mode')
  activate('single_chart_official')
  assert(#BBT.selectorLog==1 and BBT.selectorLog[1].source=='official'
    and BBT.selectorLog[1].mode=='verify',
    'Find Local Freeplay selection must verify the locked chart without replacing it')
  reset(); BBT.context.sessionId='player-2'; participants[3].verified=false
  participants[3].ready=false; online.selectedSessionId='player-2'
  activate('session_local_chart'); activate('single_chart_custom')
  assert(#BBT.selectorLog==1 and BBT.selectorLog[1].source=='custom'
    and BBT.selectorLog[1].mode=='verify',
    'Find Local Custom selection must verify the locked chart without replacing it')

  reset(); roomFixture.lifecycle='set_complete'; roomFixture.currentSetlistIndex=2
  for _,entry in ipairs(roomFixture.setlist) do entry.completed=true end
  activate('session_primary')
  assert(online.modal and online.modal.kind=='chart_source',
    'Select Next Chart must use the same single-chart source dialog')
  activate('single_chart_custom')
  assert(online.modal and online.modal.kind=='confirm' and #BBT.selectorLog==0,
    'Replacing a nonempty ordered set must confirm before leaving Online')
  activate('modal_cancel')
  assert(#BBT.selectorLog==0 and #roomFixture.setlist==3,
    'Cancelling replacement must preserve the ordered queue and avoid SongSelect')
  activate('session_primary'); activate('single_chart_custom'); activate('modal_confirm')
  assert(#BBT.selectorLog==1 and BBT.selectorLog[1].source=='custom'
    and BBT.selectorLog[1].mode=='single',
    'Confirmed replacement must open the one-off custom selector')

  reset(); online.workspace='setlist'
  activate('setlist_add_official')
  assert(BBT.selectorLog[1].source=='official' and BBT.selectorLog[1].mode=='setlist',
    'Setlist Add Official must append instead of replacing the ordered set')
  reset(); online.workspace='setlist'
  activate('setlist_add_custom')
  assert(BBT.selectorLog[1].source=='custom' and BBT.selectorLog[1].mode=='setlist',
    'Setlist Add Custom must append instead of replacing the ordered set')

  reset(); BBT.context.sessionId='player-2'; participants[3].verified=false
  participants[3].ready=false; online.selectedSessionId='player-2'; online:drawState()
  activate('participant_freeplay')
  assert(BBT.selectorLog[1].source=='official' and BBT.selectorLog[1].mode=='verify',
    'Chart validation must expose Beatblock Freeplay as an explicit source')
  BBT.selectorLog={}; online:drawState(); activate('participant_locate')
  assert(BBT.selectorLog[1].source=='custom' and BBT.selectorLog[1].mode=='verify',
    'Chart validation must retain custom chart selection beside Freeplay')

  reset(); roomFixture.lifecycle='playing'; online.workspace='setlist'; online:drawState()
  for _,id in ipairs({
    'setlist_add_official','setlist_add_custom','setlist_up','setlist_down','setlist_remove',
  }) do
    assert(not optionalControl(id),'Setlist editing must lock during play: '..id)
  end
  reset(); BBT.context.sessionId='player-2'; online.workspace='setlist'; online:drawState()
  for _,id in ipairs({
    'setlist_add_official','setlist_add_custom','setlist_up','setlist_down','setlist_remove',
  }) do
    assert(not optionalControl(id),'Non-host Setlist must remain read-only: '..id)
  end

  reset(); roomFixture.lifecycle='results'; roomFixture.setlist[1].completed=true
  online.workspace='setlist'; online:drawState()
  local nextChartLabels=0
  for _,entry in ipairs(BBT.layoutAudit.text or {}) do
    if entry.value=='NEXT CHART' then nextChartLabels=nextChartLabels+1 end
  end
  assert(nextChartLabels==1 and not optionalControl('setlist_next'),
    'Results must expose exactly one global Next Chart action')

  reset(); roomFixture.setlist=setlistOf(4); online.workspace='setlist'
  online.selectedSetlistEntryId='set-3'; online.setlistSelection=3; online:drawState()
  local controlOrder={}
  for index,control in ipairs(online.controls) do controlOrder[control.id]=index end
  assert(controlOrder['setlist_entry_set-1']<controlOrder.setlist_add_official
    and controlOrder.setlist_add_official<controlOrder.setlist_add_custom
    and controlOrder.setlist_add_custom<controlOrder.setlist_up
    and controlOrder.setlist_up<controlOrder.setlist_down
    and controlOrder.setlist_down<controlOrder.setlist_remove,
    'Setlist focus must follow rows, add actions, then selected-row actions')

  reset(); online.workspace='broadcast'; online.broadcastAdvanced=true
  online.broadcastDraft={mode='clean',width=1280,height=720,fps=60,delayMs=500}
  activate('broadcast_mode_full'); activate('broadcast_size_1080')
  activate('broadcast_fps_30'); activate('broadcast_delay_1000'); activate('broadcast_apply')
  local exportCommand=BBT.commandLog[#BBT.commandLog]
  assert(exportCommand.kind=='renderer.configure','Advanced export Apply must configure the selected renderer')
  assert(exportCommand.payload.mode=='full' and exportCommand.payload.width==1920 and exportCommand.payload.height==1080,'Advanced export must preserve mode and resolution')
  assert(exportCommand.payload.fps==30 and exportCommand.payload.delayMs==1000,'Advanced export must preserve FPS and delay')
  roomFixture.lifecycle='playing'
  -- Disabled controls are intentionally omitted from the focus/click table.
  local lockedApply=optionalControl('broadcast_apply')
  assert(not lockedApply or lockedApply.run==nil,'Advanced export Apply must lock during an active race')

  reset(); roomFixture.lifecycle='results'
  activate('session_primary')
  assert(BBT.commandLog[1].kind=='setlist.advance','Results Next Chart must advance the authoritative set')
  assert(online.workspace=='setlist','Results Next Chart must show chart selection progress in Setlist')

  online:leave()
  roomFixture.lifecycle='results'
  online:init()
  assert(online.workspace=='setlist','A host returning from native Results must land on Setlist')
  -- Disabled actions must not enter the focus/click dispatch table. A
  -- non-host still sees the host's Setlist controls for context.
  BBT.context.sessionId='player-2'
  online.workspace='setlist'
  online:drawState()
  for _,control in ipairs(online.controls) do
    assert(control.id~='setlist_add_official','disabled Setlist add action remained interactive')
    assert(control.id~='setlist_add_custom','disabled Setlist add action remained interactive')
    assert(control.id~='setlist_up' and control.id~='setlist_down'
      and control.id~='setlist_remove','disabled Setlist row action remained interactive')
  end
  -- Allowed Unicode names can exceed a byte-based label budget. Opening a
  -- destructive confirmation must preserve valid UTF-8.
  BBT.context.sessionId='host-1'
  local originalName=participants[3].displayName
  participants[3].displayName=string.rep(string.char(231,149,140),11)
  online.workspace='room'; online.selectedSessionId=participants[3].sessionId
  online:drawState()
  for _,control in ipairs(online.controls) do
    if control.id=='participant_remove' then control.run(); break end
  end
  local utf8=require('utf8')
  assert(online.modal and utf8.len(online.modal.message),
    'bounded participant confirmation contains malformed UTF-8')
  participants[3].displayName=originalName
  online.modal=nil
  online:init()
  if autorun then
    auditFile=assert(io.open(output..'/layout-audit.txt','wb'))
    beginNext()
  end
end

function love.update(dt)
  -- Online moves focus onto whatever control the pointer is over, so sampling
  -- the real cursor made the "deterministic" captures depend on where the
  -- physical mouse happened to rest: any scenario with an enabled control under
  -- that point recorded a different focused button. Park the pointer off-canvas
  -- for autorun so focus comes only from focusId, and keep the live cursor for
  -- the interactive harness a reviewer drives by hand.
  if autorun then mouse.rx=-1000; mouse.ry=-1000
  else local x,y=love.mouse.getPosition(); mouse.rx=x; mouse.ry=y end
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
