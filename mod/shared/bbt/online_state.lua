-- Beatblock-native adaptive Online dashboard. Runtime and room behavior stay
-- in the native engine; this state only normalizes snapshots and presents the
-- next useful action without making players hunt through parallel pages.
local Dashboard = require('bbt.dashboard_model')

-- These are palette-index source colors consumed by Beatblock's fixed shader.
local C = {
  black={0,0,0,1}, panel={0,0,0,1}, raised={1,0,0,1},
  white={1,1,1,1}, muted={1,1,1,.68}, disabled={1,1,1,.48}, dimBlack={0,0,0,.55},
  red={0,0,1,1}, yellow={0,1,0,1}, green={1,1,0,1}, cyan={1,0,1,1}, blue={0,1,1,1},
}
local UTILITIES = {
  {id='setlist',label='SETLIST'}, {id='obs',label='SPECTATE + OBS'},
  {id='history',label='HISTORY'}, {id='settings',label='SETTINGS'},
}
local STREAM_IDS = {'A','B','C','D'}

local function setc(value,alpha) love.graphics.setColor(value[1],value[2],value[3],alpha or value[4] or 1) end
local function tone(name) return C[name] or C.white end
local function hit(x,y,w,h)
  local mx,my=mouse and mouse.rx or -1,mouse and mouse.ry or -1
  return mx>=x and mx<=x+w and my>=y and my<=y+h
end
local function clicked(x,y,w,h) return mouse and mouse.pressed==1 and hit(x,y,w,h) end
local function pressed(name)
  if not maininput or not maininput.pressed then return false end
  local ok,value=pcall(maininput.pressed,maininput,name)
  return ok and value==true
end
local function accept() return pressed('select') or pressed('accept') end
local function sound(name) if te and sounds and sounds[name] then pcall(te.play,sounds[name],'static','sfx',.35) end end
local function room() return BBT.lastLobby and BBT.lastLobby.id~='offline' and BBT.lastLobby.lifecycle~='closed' and BBT.lastLobby or nil end
local function host() return BBT.isOrganizer() end
local function locked(current) return current and (current.lifecycle=='countdown' or current.lifecycle=='playing') end
local function clamp(value,low,high) return math.max(low,math.min(high,value)) end
local function short(value,length)
  value=tostring(value or '')
  if #value<=length then return value end
  return value:sub(1,math.max(1,length-1))..'~'
end

local function context()
  local me=BBT.currentPlayer()
  local chartVerified=BBT.chartVerified
  -- An explicit participant verification result wins over the older local
  -- convenience flag, including when that result is false.
  if me and me.verified~=nil then chartVerified=me.verified end
  return {
    room=room(), me=me, isHost=host(), chartVerified=chartVerified,
    runtimeReady=BBT.companionConnected, runtimeStarting=BBT.runtimeStarting,
  }
end

local function panel(x,y,w,h)
  setc(C.panel); love.graphics.rectangle('fill',x,y,w,h,3,3)
  setc(C.raised); love.graphics.rectangle('line',x+.5,y+.5,w-1,h-1,3,3)
end

local function button(x,y,w,h,label,active,buttonTone,enabled)
  local available=enabled~=false
  local fill=available and (active and tone(buttonTone or 'cyan') or C.raised) or C.disabled
  setc(fill); love.graphics.rectangle('fill',x,y,w,h,2,2)
  if active then setc(C.white); love.graphics.rectangle('line',x+.5,y+.5,w-1,h-1,2,2) end
  setc(available and C.black or C.dimBlack)
  love.graphics.printf(label,x+4,y+math.floor((h-9)/2)+1,w-8,'center')
end

local function chip(x,y,w,label,chipTone)
  setc(tone(chipTone)); love.graphics.rectangle('line',x+.5,y+.5,w-1,16,2,2)
  love.graphics.printf(label,x+3,y+5,w-6,'center')
end

local function leaveToMenu(self)
  local music=self.menuMusicManager
  if music then music:clearOnBeatHooks() end
  local previous=cs; cs=bs.load('Menu')
  if previous and previous.leave then previous:leave() end
  cs.menuMusicManager=music
  -- Reusing the active manager prevents the intro cue from replaying.
  cs:init()
end

local function exitOnline(self)
  BBT.exitOnline()
  leaveToMenu(self)
end

local function addAction(list,id,label,description,actionTone,run,enabled)
  list[#list+1]={id=id,label=label,description=description,tone=actionTone or 'white',run=run,enabled=enabled~=false}
end

local function copyJoinLink()
  if not love.system then return end
  local address=BBT.settings and BBT.settings.hostAddress or '127.0.0.1'
  local port=BBT.settings and BBT.settings.hostPort or 32145
  love.system.setClipboardText('bbt://'..tostring(address)..':'..tostring(port)..'?v=2')
end

local function ensureRoster(self)
  local participants=room() and room().participants or {}
  if #participants==0 then self.rosterSelection=1; self.rosterOffset=0; return participants end
  self.rosterSelection,self.rosterOffset=Dashboard.scroll(self.rosterSelection,self.rosterOffset,#participants,0,8)
  return participants
end

local function selectedParticipant(self)
  local participants=ensureRoster(self)
  return participants[self.rosterSelection]
end

local function requestConfirm(self,title,message,label,run)
  self.confirm={title=title,message=message,label=label or 'CONFIRM',run=run}
  self.confirmChoice=2
end

local function primaryAction(self)
  return Dashboard.primary(context())
end

local function runPrimary(self,item)
  local current=room()
  if item.id=='host_room' then self:openForm('host')
  elseif item.id=='open_installer' then BBT.openInstaller()
  elseif item.id=='select_chart' then self.overlay='setlist'; self.overlayFocus='actions'; self.overlayActionSelection=1
  elseif item.id=='locate_chart' and current and current.chart then
    if current.chart.official then BBT.openOfficialSelect('verify') else BBT.openChartSelect('verify') end
  elseif item.id=='ready' then BBT.command('room.ready_request',{ready=true})
  elseif item.id=='start_race' then BBT.command('room.start_request',{force=false})
  elseif item.id=='advance_set' then BBT.command('setlist.advance',{})
  elseif item.id=='view_results' then self.overlay='history'; self.overlayFocus='list'; self.overlayActionSelection=1 end
end

local function secondaryActions(self)
  local list,current,me={},room(),BBT.currentPlayer()
  if not current then
    addAction(list,'join','JOIN ROOM','Connect as a verified player.','cyan',function() self:openForm('join',false) end,BBT.companionConnected)
    addAction(list,'spectate','JOIN AS SPECTATOR','Follow room telemetry and rankings.','white',function() self:openForm('join',true) end,BBT.companionConnected)
    addAction(list,'exit','EXIT ONLINE','Stop local Online session services.','red',function()
      requestConfirm(self,'EXIT ONLINE','Leave Online and stop the runtime, API, exports, and renderers?','EXIT',function() exitOnline(self) end)
    end)
  else
    addAction(list,'room_options','ROOM OPTIONS','Open room sharing and session controls.','white',function() self.sideMenu='room'; self.sideSelection=1 end)
  end
  return list
end

local function participantActions(self,target)
  local list,current={},room()
  if not target or not current then return list end
  if host() and target.sessionId~=current.hostSessionId then
    if not target.admitted then
      addAction(list,'approve','APPROVE','Admit this participant to the room.','green',function()
        BBT.command('room.admission_set',{sessionId=target.sessionId,admit=true,role=target.role}); self.sideMenu=nil
      end)
      addAction(list,'reject','REJECT','Reject this admission request.','red',function()
        requestConfirm(self,'REJECT REQUEST','Reject '..short(target.displayName,24)..' from this room?','REJECT',function()
          BBT.command('room.admission_set',{sessionId=target.sessionId,admit=false,role=target.role}); self.sideMenu=nil
        end)
      end)
    else
      addAction(list,'role','CHANGE ROLE','Switch between player and spectator.','yellow',function()
        BBT.command('room.role_set',{sessionId=target.sessionId,role=target.role=='spectator' and 'player' or 'spectator'}); self.sideMenu=nil
      end,not locked(current))
      addAction(list,'kick','REMOVE','Remove this participant from the room.','red',function()
        requestConfirm(self,'REMOVE PLAYER','Remove '..short(target.displayName,24)..' from the room?','REMOVE',function()
          BBT.command('room.kick',{sessionId=target.sessionId}); self.sideMenu=nil
        end)
      end,not locked(current))
    end
  end
  addAction(list,'close','CLOSE','Return to the room dashboard.','white',function() self.sideMenu=nil end)
  return list
end

local function roomOptionActions(self)
  local list,current,me={},room(),BBT.currentPlayer()
  if not current then return list end
  if host() then
    addAction(list,'copy','COPY JOIN LINK','Copy a bbt:// link without the password.','cyan',copyJoinLink)
    addAction(list,'force','FORCE START','Launch assigned clients and record incomplete runs as DNF.','yellow',function()
      requestConfirm(self,'FORCE START','Start before every player is verified and ready?','FORCE START',function()
        BBT.command('room.start_request',{force=true}); self.sideMenu=nil
      end)
    end,current.chart~=nil and not locked(current))
    addAction(list,'close_room','CLOSE ROOM','Close the room for every participant.','red',function()
      requestConfirm(self,'CLOSE ROOM','Close this room and disconnect every participant?','CLOSE ROOM',function()
        BBT.command('room.close_request',{}); self.sideMenu=nil
      end)
    end,not locked(current))
  else
    if me and me.ready and not locked(current) then addAction(list,'unready','UNREADY','Return to waiting.','yellow',function() BBT.command('room.ready_request',{ready=false}); self.sideMenu=nil end) end
    addAction(list,'leave','LEAVE ROOM','Disconnect while keeping Online available.','red',function()
      requestConfirm(self,'LEAVE ROOM','Disconnect from '..short(current.name,28)..'?','LEAVE',function()
        BBT.command('room.leave_request',{}); self.sideMenu=nil
      end)
    end,not locked(current))
  end
  addAction(list,'exit','EXIT ONLINE','Leave the room and stop local Online services.','red',function()
    requestConfirm(self,'EXIT ONLINE','Leave Online and stop the runtime, API, exports, and renderers?','EXIT',function() exitOnline(self) end)
  end)
  addAction(list,'cancel','BACK','Close room options.','white',function() self.sideMenu=nil end)
  return list
end

local function overlayActions(self)
  local list,current={},room()
  if self.overlay=='setlist' then
    if current and host() then
      addAction(list,'official','SELECT ATOM MAP','Lock an official Beatblock chart.','cyan',function() BBT.openOfficialSelect('host') end)
      addAction(list,'custom','SELECT CUSTOM','Lock a custom chart package.','cyan',function() BBT.openChartSelect('host') end)
      addAction(list,'add_official','ADD OFFICIAL','Append an official chart to the set.','green',function() BBT.openOfficialSelect('setlist') end)
      addAction(list,'add_custom','ADD CUSTOM','Append a custom chart to the set.','green',function() BBT.openChartSelect('setlist') end)
      local count=#(current.setlist or {}); local index=self.setlistSelection or 1
      addAction(list,'up','MOVE UP','Move the selected chart earlier.','white',function() BBT.command('setlist.move',{from=index-1,to=math.max(0,index-2)}) end,count>1 and index>1)
      addAction(list,'down','MOVE DOWN','Move the selected chart later.','white',function() BBT.command('setlist.move',{from=index-1,to=math.min(count-1,index)}) end,count>1 and index<count)
      addAction(list,'remove','REMOVE','Remove the selected setlist chart.','red',function()
        requestConfirm(self,'REMOVE CHART','Remove this chart from the setlist?','REMOVE',function() BBT.command('setlist.remove',{index=index-1}) end)
      end,count>0)
    elseif current and current.chart then
      addAction(list,'locate','LOCATE CHART','Select the exact chart locked by the host.','cyan',function()
        if current.chart.official then BBT.openOfficialSelect('verify') else BBT.openChartSelect('verify') end
      end)
    end
  elseif self.overlay=='obs' then
    local target=selectedParticipant(self)
    for index,id in ipairs(STREAM_IDS) do
      addAction(list,'assign_'..id,'ASSIGN STREAM '..id,'Assign the selected participant to stable Stream '..id..'.','cyan',function()
        BBT.command('renderer.configure',{slot=id,participantId=target and target.sessionId or '',participantName=target and target.displayName or '',mode='clean',width=1280,height=720,fps=60,delayMs=(BBT.settings and BBT.settings.spectatorDelayMs) or 500,featured=index==1})
      end,host() and target~=nil)
    end
    local id=STREAM_IDS[self.streamSelection or 1]
    addAction(list,'feature','FEATURE '..id,'Drive shared audio and featured text exports.','green',function() BBT.command('renderer.configure',{slot=id,featured=true}) end,host())
    addAction(list,'stop','STOP '..id,'Stop the selected stable stream slot.','red',function() BBT.command('renderer.stop',{slot=id}) end,host())
    addAction(list,'exports','OPEN EXPORTS','Open atomic OBS text exports.','white',function() BBT.command('paths.open_exports',{}) end)
  elseif self.overlay=='history' then
    local history=BBT.history or {}; local item=history[self.historySelection or 1]
    addAction(list,'refresh','REFRESH','Reload saved match summaries.','cyan',function() BBT.command('history.list',{}) end)
    addAction(list,'delete','DELETE RESULT','Delete the selected summary and journal.','red',function()
      requestConfirm(self,'DELETE RESULT','Permanently delete this saved result and its raw events?','DELETE',function() if item then BBT.command('history.delete',{roomId=item.id}) end end)
    end,item~=nil)
    addAction(list,'prune','PRUNE EVENTS','Delete raw event journals older than 30 days.','yellow',function()
      requestConfirm(self,'PRUNE EVENTS','Delete raw journals older than 30 days while keeping summaries?','PRUNE',function() BBT.command('history.prune',{days=30}) end)
    end)
  elseif self.overlay=='settings' then
    addAction(list,'hud',BBT.hudEnabled and 'DISABLE HUD' or 'ENABLE HUD','Toggle the minimal online gameplay HUD.','cyan',function()
      BBT.hudEnabled=not BBT.hudEnabled; BBT.command('settings.update',{hudEnabled=BBT.hudEnabled})
    end)
    addAction(list,'refresh','REFRESH STATUS','Request current runtime and network diagnostics.','white',function() BBT.command('diagnostics.get',{}) end)
    addAction(list,'logs','OPEN LOGS','Open runtime diagnostic logs.','white',function() BBT.command('paths.open_logs',{}) end)
    addAction(list,'exports','OPEN EXPORTS','Open atomic OBS exports.','white',function() BBT.command('paths.open_exports',{}) end)
    addAction(list,'token','ROTATE API TOKEN','Invalidate the localhost API token.','yellow',function()
      requestConfirm(self,'ROTATE TOKEN','Existing local API clients will disconnect. Rotate the token?','ROTATE',function() BBT.command('api.token_rotate',{}) end)
    end)
    addAction(list,'restart','RESTART RUNTIME','Restart local services; an active run becomes invalid.','yellow',function()
      requestConfirm(self,'RESTART RUNTIME','Restart Online services now?','RESTART',function() BBT.command('runtime.restart_request',{}) end)
    end)
    addAction(list,'installer','OPEN INSTALLER','Open installation and repair tools.','white',function() BBT.openInstaller() end)
  end
  return list
end

local KEYS={}
for c=string.byte('a'),string.byte('z') do KEYS[string.char(c)]=string.char(c) end
for n=0,9 do KEYS[tostring(n)]=tostring(n) end
KEYS.space=' '; KEYS.period='.'; KEYS.minus='-'; KEYS.semicolon=':'

local function formFields(mode)
  if mode=='host' then return {
    {id='name',label='ROOM NAME',max=40}, {id='port',label='UDP PORT',max=5},
    {id='password',label='PASSWORD',max=128,secret=true}, {id='displayName',label='DISPLAY NAME',max=48},
    {id='approval',label='HOST APPROVAL',toggle=true},
  } end
  return {{id='address',label='HOST IP:PORT',max=80},{id='password',label='PASSWORD',max=128,secret=true},{id='displayName',label='DISPLAY NAME',max=48}}
end

local function openForm(self,mode,spectator)
  self.formMode=mode; self.formSpectator=spectator==true; self.formField=1; self.keyLatch={}
  self.formValues=mode=='host' and {name='Beatblock Room',port='32145',password='',displayName=BBT.context.playerName or 'Host',approval=true}
    or {address='127.0.0.1:32145',password='',displayName=BBT.context.playerName or 'Player'}
  if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(true) end
end

local function closeForm(self)
  self.formMode=nil
  if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(false) end
end

local function editForm(self,text)
  local field=formFields(self.formMode)[self.formField]
  if not field or field.toggle then return end
  local value=self.formValues[field.id] or ''
  self.formValues[field.id]=text=='\b' and value:sub(1,-2) or (#value<field.max and value..text or value)
end

local function submitForm(self)
  local value=self.formValues
  if value.password=='' then BBT.lastError='A room password is required.'; return end
  if value.displayName=='' then BBT.lastError='A display name is required.'; return end
  BBT.context.playerName=value.displayName
  if self.formMode=='host' then
    local port=tonumber(value.port)
    if not port or port<1 or port>65535 then BBT.lastError='UDP port must be 1-65535.'; return end
    if value.name=='' then BBT.lastError='A room name is required.'; return end
    BBT.command('room.host_request',{name=value.name,port=port,password=value.password,displayName=value.displayName,hostApproval=value.approval})
  else
    BBT.command('room.join_request',{address=value.address,password=value.password,displayName=value.displayName,spectator=self.formSpectator})
  end
  closeForm(self)
end

local function updateForm(self)
  local fields=formFields(self.formMode); local submitIndex=#fields+1
  for key,text in pairs(KEYS) do local down=love.keyboard.isDown(key); if down and not self.keyLatch[key] then editForm(self,text) end; self.keyLatch[key]=down end
  local backspace=love.keyboard.isDown('backspace'); if backspace and not self.keyLatch.backspace then editForm(self,'\b') end; self.keyLatch.backspace=backspace
  if pressed('menu_up') then self.formField=clamp(self.formField-1,1,submitIndex); sound('click') end
  if pressed('menu_down') then self.formField=clamp(self.formField+1,1,submitIndex); sound('click') end
  local field=fields[self.formField]
  if field and field.toggle and (pressed('menu_left') or pressed('menu_right')) then self.formValues[field.id]=not self.formValues[field.id]; sound('click') end
  for index=1,#fields do
    local y=78+(index-1)*34
    if clicked(74,y,452,29) then self.formField=index; if fields[index].toggle then self.formValues[fields[index].id]=not self.formValues[fields[index].id] end end
  end
  if clicked(178,270,244,28) then self.formField=submitIndex; submitForm(self); return end
  if accept() then
    if self.formField==submitIndex then submitForm(self)
    elseif field and field.toggle then self.formValues[field.id]=not self.formValues[field.id]
    else self.formField=math.min(submitIndex,self.formField+1) end
    sound('hold')
  elseif pressed('back') then closeForm(self) end
end

local function drawHeader(self)
  setc(C.black); love.graphics.rectangle('fill',0,0,project.res.x,project.res.y)
  local current=room()
  local heading=current and ('BBT  /  '..short(current.name or 'ROOM',24)) or 'BEATBLOCK TOGETHER'
  love.graphics.setFont(fonts.main); setc(C.white); love.graphics.print(heading,12,8)
  love.graphics.setFont(fonts.digitalDisco)
  local lifecycle=current and string.upper(current.lifecycle or 'forming') or 'DIRECT-IP'
  setc(BBT.companionConnected and C.green or C.yellow)
  love.graphics.printf((BBT.companionConnected and 'LINK READY' or string.upper(BBT.runtimeLaunchStatus or 'STARTING'))..'  /  '..lifecycle,292,10,250,'right')
  button(550,5,38,22,'? HELP',self.focusZone=='help','cyan',true)
end

local function drawChartStrip()
  local current=room(); local chart=current and current.chart
  panel(12,34,576,38); love.graphics.setFont(fonts.digitalDisco)
  setc(C.muted); love.graphics.print(chart and 'CURRENT CHART' or 'ONLINE SESSION',20,41)
  setc(C.white); love.graphics.print(short(chart and chart.songName or 'Direct-IP room setup',38),20,55)
  if chart then
    local me=BBT.currentPlayer(); local verified=me and me.verified or BBT.chartVerified
    if me and me.verified~=nil then verified=me.verified end
    setc(verified and C.green or C.yellow); love.graphics.printf(verified and 'VERIFIED' or 'VERIFY CHART',400,42,178,'right')
    setc(C.muted); love.graphics.printf(short(chart.variant or 'Default',20),400,57,178,'right')
  else
    setc(BBT.companionConnected and C.green or C.yellow); love.graphics.printf(BBT.companionConnected and 'READY TO CONNECT' or 'RUNTIME STARTING',372,51,206,'right')
  end
end

local function drawConnect(self)
  panel(12,78,356,208); panel(376,78,212,208); love.graphics.setFont(fonts.digitalDisco)
  setc(C.white); love.graphics.print('PLAY ONLINE',24,90)
  setc(C.muted); love.graphics.printf('Create a room on this computer or connect directly to a host. Room passwords stay out of snapshots and logs.',24,112,330,'left')
  local status=BBT.companionConnected and 'ONLINE SERVICES READY' or string.upper(BBT.runtimeLaunchStatus or 'STARTING ONLINE SERVICES')
  setc(BBT.companionConnected and C.green or C.yellow); love.graphics.print(status,24,174)
  setc(C.muted); love.graphics.print('UDP / QUIC',24,204); love.graphics.printf(tostring(BBT.settings and BBT.settings.hostPort or 32145),202,204,150,'right')
  love.graphics.print('LOCAL API',24,226); love.graphics.printf(BBT.companionConnected and '127.0.0.1:8974' or 'WAITING',202,226,150,'right')
  love.graphics.print('OBS EXPORTS',24,248); love.graphics.printf(BBT.companionConnected and 'ACTIVE' or 'WAITING',202,248,150,'right')

  local primary=primaryAction(self); setc(C.muted); love.graphics.print('NEXT ACTION',388,90)
  button(388,110,188,36,primary.label,self.focusZone=='primary',primary.tone,primary.enabled)
  setc(C.muted); love.graphics.printf(primary.description,388,153,188,'left')
  local secondary=secondaryActions(self); self.secondary=secondary
  for index,item in ipairs(secondary) do
    button(388,205+(index-1)*23,188,19,item.label,self.focusZone=='secondary' and self.secondarySelection==index,item.tone,item.enabled)
  end
end

local function drawRoster(self)
  local participants=ensureRoster(self); local summary=Dashboard.summary(context())
  panel(12,78,356,208); love.graphics.setFont(fonts.digitalDisco)
  setc(C.white); love.graphics.print('ROOM ROSTER',22,88)
  setc(C.muted); love.graphics.printf(summary.ready..'/'..summary.players..' READY  /  '..summary.spectators..' WATCHING',162,89,194,'right')
  love.graphics.print('PLAYER',22,108); love.graphics.print('STATE',182,108); love.graphics.printf('SCORE',288,108,68,'right')
  setc(C.raised); love.graphics.line(22,120,356,120)
  for row=1,8 do
    local index=self.rosterOffset+row; local participant=participants[index]; local y=125+(row-1)*19
    if participant then
      local active=index==self.rosterSelection
      if active then setc(C.raised); love.graphics.rectangle('fill',19,y-3,340,17,2,2) end
      local status,statusTone=Dashboard.participantStatus(participant)
      setc(active and C.black or C.white)
      local role=participant.role=='spectator' and '[S] ' or (participant.sessionId==(room() and room().hostSessionId) and '[H] ' or '[P] ')
      love.graphics.print(role..short(participant.displayName or 'Player',18),22,y)
      setc(active and C.black or tone(statusTone)); love.graphics.print(status,182,y)
      setc(active and C.black or C.white)
      local score=participant.validity=='dnf' and 'DNF' or string.format('%.2f',participant.accuracy or 100)
      if participant.rank then score='#'..participant.rank..'  '..score end
      love.graphics.printf(score,276,y,80,'right')
    end
  end
  setc(C.muted)
  local first=#participants==0 and 0 or self.rosterOffset+1; local last=math.min(#participants,self.rosterOffset+8)
  love.graphics.printf(first..'-'..last..' / '..#participants,270,270,86,'right')
end

local function drawControl(self)
  panel(376,78,212,208); love.graphics.setFont(fonts.digitalDisco)
  local me=BBT.currentPlayer(); local current=room(); local summary=Dashboard.summary(context())
  setc(C.muted); love.graphics.print(host() and 'HOST CONTROL' or 'YOUR STATUS',388,90)
  setc(C.white); love.graphics.print(short(me and me.displayName or 'WAITING FOR IDENTITY',22),388,108)
  if me then
    local status,statusTone=Dashboard.participantStatus(me); chip(388,125,88,status,statusTone)
    chip(482,125,94,string.upper(me.role or 'player'),me.role=='spectator' and 'cyan' or 'white')
  end
  local primary=primaryAction(self)
  button(388,151,188,36,primary.label,self.focusZone=='primary',primary.tone,primary.enabled)
  setc(C.muted); love.graphics.printf(primary.description,388,194,188,'left')
  if current and current.lifecycle=='results' then
    setc(C.green); love.graphics.printf('RESULTS RECEIVED',388,235,188,'center')
  elseif host() and summary.pending>0 then
    setc(C.yellow); love.graphics.printf(summary.pending..' ADMISSION REQUEST'..(summary.pending==1 and '' or 'S'),388,232,188,'center')
  end
  local secondary=secondaryActions(self); self.secondary=secondary
  for index,item in ipairs(secondary) do
    button(388,253+(index-1)*23,188,19,item.label,self.focusZone=='secondary' and self.secondarySelection==index,item.tone,item.enabled)
  end
end

local function drawUtilityBar(self)
  love.graphics.setFont(fonts.digitalDisco)
  for index,item in ipairs(UTILITIES) do
    local x=12+(index-1)*144
    button(x,292,140,28,item.label,self.focusZone=='utility' and self.utilitySelection==index,'cyan',true)
  end
end

local function footerDescription(self)
  if BBT.lastError then return tostring(BBT.lastError),'red' end
  if self.focusZone=='roster' then
    local participant=selectedParticipant(self)
    return participant and ('Open '..short(participant.displayName,28)..' for role, verification, and run details.') or 'Participants appear here after joining.','white'
  end
  if self.focusZone=='primary' then return primaryAction(self).description,'white' end
  if self.focusZone=='secondary' then local item=self.secondary and self.secondary[self.secondarySelection]; return item and item.description or 'Room controls.','white' end
  if self.focusZone=='utility' then return 'Open '..UTILITIES[self.utilitySelection].label..' without leaving the room dashboard.','white' end
  return 'Open help for the current room state and controls.','white'
end

local function drawFooter(self)
  love.graphics.setFont(fonts.digitalDisco); local description,descriptionTone=footerDescription(self)
  setc(tone(descriptionTone)); love.graphics.printf(short(description,92),12,326,576,'center')
  setc(C.muted); love.graphics.printf('ARROWS NAVIGATE   ENTER SELECT   ESC BACK',12,346,576,'center')
end

local function drawOverlayList(self)
  local current=room(); love.graphics.setFont(fonts.digitalDisco)
  if self.overlay=='setlist' then
    setc(C.muted); love.graphics.print('ORDER',42,78); love.graphics.print('CHART',82,78)
    local set=current and current.setlist or {}
    self.setlistSelection,self.setlistOffset=Dashboard.scroll(self.setlistSelection,self.setlistOffset,#set,0,10)
    for row=1,math.min(10,#set) do
      local index=(self.setlistOffset or 0)+row
      local item=set[index]; local y=96+(row-1)*18; local active=self.overlayFocus=='list' and self.setlistSelection==index
      if active then setc(C.raised); love.graphics.rectangle('fill',38,y-3,330,17,2,2) end
      setc(active and C.black or (item.completed and C.green or C.white)); love.graphics.print(tostring(index),46,y)
      love.graphics.print(short(item.chart and item.chart.songName or 'Chart',31),82,y)
    end
    if #set==0 then setc(C.muted); love.graphics.printf('Use the chart actions to lock one chart or build an ordered set.',42,116,320,'left') end
  elseif self.overlay=='obs' then
    for index,id in ipairs(STREAM_IDS) do
      local stream=(BBT.renderers or {})[index] or {}; local y=84+(index-1)*47; local active=self.overlayFocus=='list' and self.streamSelection==index
      if active then setc(C.raised); love.graphics.rectangle('fill',38,y-3,330,40,2,2) end
      setc(active and C.black or (stream.healthy and C.green or (stream.active and C.yellow or C.white))); love.graphics.print('STREAM '..id,46,y)
      love.graphics.printf(short(stream.participantName or 'UNASSIGNED',24),148,y,210,'right')
      local health=stream.healthy and 'LIVE' or (stream.active and 'STARTING' or 'STOPPED')
      local detail=stream.lastError and short(stream.lastError,42) or (health..'  '..tostring(stream.fps or 60)..'fps  DROP '..tostring(stream.droppedFrames or 0))
      setc(active and C.dimBlack or (stream.lastError and C.red or C.muted)); love.graphics.printf(detail,46,y+18,312,'right')
    end
  elseif self.overlay=='history' then
    local history=BBT.history or {}
    self.historySelection,self.historyOffset=Dashboard.scroll(self.historySelection,self.historyOffset,#history,0,10)
    for row=1,math.min(10,#history) do
      local index=(self.historyOffset or 0)+row
      local item=history[index]; local y=86+(row-1)*19; local active=self.overlayFocus=='list' and self.historySelection==index
      if active then setc(C.raised); love.graphics.rectangle('fill',38,y-3,330,17,2,2) end
      setc(active and C.black or C.white); love.graphics.print(short(item.name or 'Match',28),46,y)
      setc(active and C.dimBlack or C.muted); love.graphics.printf(string.upper(item.lifecycle or 'results'),274,y,84,'right')
    end
    if #history==0 then setc(C.muted); love.graphics.print('Completed match summaries appear here.',46,104) end
  else
    local diagnostics=BBT.diagnostics or {}
    local lines={
      {'LIVE HUD',BBT.hudEnabled and 'ENABLED' or 'DISABLED'}, {'PROTOCOL','V'..tostring(diagnostics.protocolVersion or 2)},
      {'RUNTIME',diagnostics.runtimeVersion or BBT.version}, {'CONNECTION',BBT.connected and 'CONNECTED' or 'IDLE'},
      {'PEERS',tostring(diagnostics.peerCount or 0)}, {'RENDER BUDGET',diagnostics.rendererBudgetWarning or 'OK'},
      {'LOCAL API',BBT.companionConnected and '127.0.0.1:8974' or 'OFFLINE'},
    }
    for index,item in ipairs(lines) do
      local y=84+(index-1)*27; setc(C.muted); love.graphics.print(item[1],46,y)
      local good=item[2]=='OK' or item[2]=='ENABLED' or item[2]=='CONNECTED'
      setc(good and C.green or C.white); love.graphics.printf(short(item[2],24),166,y,192,'right')
    end
  end
end

local function drawOverlay(self)
  setc(C.black,.94); love.graphics.rectangle('fill',0,28,600,332)
  panel(24,38,552,286); love.graphics.setFont(fonts.main); setc(C.white)
  local titles={setlist='SETLIST',obs='SPECTATE + OBS',history='MATCH HISTORY',settings='SETTINGS + DIAGNOSTICS'}
  love.graphics.print(titles[self.overlay],38,50)
  love.graphics.setFont(fonts.digitalDisco); button(536,46,28,20,'X',false,'red',true)
  drawOverlayList(self)
  local actions=overlayActions(self); self.overlayActions=actions
  setc(C.muted); love.graphics.print('ACTIONS',392,78)
  for index,item in ipairs(actions) do
    button(390,94+(index-1)*24,172,20,item.label,self.overlayFocus=='actions' and self.overlayActionSelection==index,item.tone,item.enabled)
  end
  local selected=actions[self.overlayActionSelection]
  setc(C.white); love.graphics.printf(selected and selected.description or 'ESC returns to the room dashboard.',38,298,524,'center')
end

local function sideActions(self)
  if self.sideMenu=='participant' then return participantActions(self,selectedParticipant(self)) end
  return roomOptionActions(self)
end

local function drawSideMenu(self)
  setc(C.black,.92); love.graphics.rectangle('fill',342,72,258,218)
  panel(348,78,240,208); love.graphics.setFont(fonts.main); setc(C.white)
  local target=selectedParticipant(self)
  love.graphics.print(self.sideMenu=='participant' and short(target and target.displayName or 'PLAYER',22) or 'ROOM OPTIONS',360,89)
  love.graphics.setFont(fonts.digitalDisco)
  if self.sideMenu=='participant' and target then
    local status,statusTone=Dashboard.participantStatus(target)
    chip(360,113,96,status,statusTone); chip(462,113,114,string.upper(target.role or 'player'),'white')
    setc(C.muted); love.graphics.print('ACCURACY',360,140); setc(C.white); love.graphics.printf(string.format('%.2f',target.accuracy or 100),476,140,100,'right')
    setc(C.muted); love.graphics.print('VERIFIED',360,157); setc(target.verified and C.green or C.yellow); love.graphics.printf(target.verified and 'YES' or 'NO',476,157,100,'right')
  end
  local actions=sideActions(self); self.sideActions=actions
  local start=self.sideMenu=='participant' and 181 or 112
  for index,item in ipairs(actions) do button(360,start+(index-1)*24,216,20,item.label,self.sideSelection==index,item.tone,item.enabled) end
end

local function helpActions(self)
  return {
    {label='OPEN LOGS',tone='white',run=function() BBT.command('paths.open_logs',{}) end},
    {label='OPEN INSTALLER',tone='yellow',run=function() BBT.openInstaller() end},
    {label='CLOSE HELP',tone='cyan',run=function() self.helpOpen=false; self.focusZone='primary' end},
  }
end

local function drawHelp(self)
  setc(C.black,.9); love.graphics.rectangle('fill',0,28,600,332)
  panel(302,34,286,314); love.graphics.setFont(fonts.main); setc(C.white); love.graphics.print('ONLINE HELP',316,48)
  love.graphics.setFont(fonts.digitalDisco); local title,copy=Dashboard.help(context(),self.overlay)
  setc(C.cyan); love.graphics.print(title,316,78)
  setc(C.white); love.graphics.printf(copy,316,100,256,'left')
  setc(C.muted); love.graphics.print('CONTROLS',316,166)
  love.graphics.printf('ARROWS  MOVE FOCUS\nENTER   SELECT / OPEN\nESC     CLOSE ONE LAYER\nMOUSE   POINT AND CLICK',316,186,256,'left')
  local actions=helpActions(self); self.helpActions=actions
  for index,item in ipairs(actions) do button(316,257+(index-1)*25,256,21,item.label,self.helpSelection==index,item.tone,true) end
end

local function drawConfirm(self)
  setc(C.black,.94); love.graphics.rectangle('fill',0,0,600,360)
  panel(112,96,376,168); love.graphics.setFont(fonts.main); setc(C.white); love.graphics.printf(self.confirm.title,126,112,348,'center')
  love.graphics.setFont(fonts.digitalDisco); setc(C.white); love.graphics.printf(self.confirm.message,132,148,336,'center')
  button(132,218,156,26,self.confirm.label,self.confirmChoice==1,'red',true)
  button(312,218,156,26,'CANCEL',self.confirmChoice==2,'cyan',true)
end

local function drawForm(self)
  setc(C.black,.95); love.graphics.rectangle('fill',0,28,600,332)
  panel(58,38,484,290); love.graphics.setFont(fonts.main); setc(C.white)
  love.graphics.printf(self.formMode=='host' and 'HOST A ROOM' or (self.formSpectator and 'JOIN AS SPECTATOR' or 'JOIN A ROOM'),58,50,484,'center')
  love.graphics.setFont(fonts.digitalDisco); local fields=formFields(self.formMode)
  for index,field in ipairs(fields) do
    local y=78+(index-1)*34; local active=self.formField==index
    setc(active and C.cyan or C.muted); love.graphics.print(field.label,74,y)
    if field.toggle then
      chip(354,y-4,172,self.formValues[field.id] and 'REQUIRED' or 'PASSWORD ONLY',self.formValues[field.id] and 'green' or 'white')
    else
      setc(C.raised); love.graphics.rectangle('fill',230,y-5,296,23,2,2)
      setc(C.black); local value=self.formValues[field.id] or ''; if field.secret then value=string.rep('*',#value) end
      love.graphics.print(short(value:sub(-42),42),238,y+1)
      if active then setc(C.white); love.graphics.rectangle('line',230.5,y-4.5,295,22,2,2) end
    end
  end
  button(178,270,244,28,self.formMode=='host' and 'CREATE ROOM' or 'CONNECT',self.formField==#fields+1,'green',true)
  setc(C.muted); love.graphics.printf('UP/DOWN FIELDS   ENTER CONTINUE   ESC CANCEL',74,307,452,'center')
end

local function drawBase(self)
  drawHeader(self); drawChartStrip()
  if Dashboard.phase(context())=='connect' or Dashboard.phase(context())=='runtime_starting' or Dashboard.phase(context())=='runtime_error' then drawConnect(self)
  else drawRoster(self); drawControl(self) end
  drawUtilityBar(self); drawFooter(self)
end

local function activate(item)
  if not item or not item.enabled then BBT.lastError='That action is unavailable in the current room state.'; return end
  item.run(); sound('hold')
end

local function updateConfirm(self)
  if pressed('menu_left') or pressed('menu_right') then self.confirmChoice=self.confirmChoice==1 and 2 or 1; sound('click') end
  if clicked(132,218,156,26) then self.confirmChoice=1 elseif clicked(312,218,156,26) then self.confirmChoice=2 end
  if accept() or clicked(132,218,156,26) or clicked(312,218,156,26) then
    local confirm=self.confirm; local choice=self.confirmChoice; self.confirm=nil
    if choice==1 then confirm.run() end
  elseif pressed('back') then self.confirm=nil end
end

local function updateHelp(self)
  local actions=helpActions(self)
  if pressed('menu_up') then self.helpSelection=(self.helpSelection-2)%#actions+1; sound('click') end
  if pressed('menu_down') then self.helpSelection=self.helpSelection%#actions+1; sound('click') end
  for index=1,#actions do if clicked(316,257+(index-1)*25,256,21) then self.helpSelection=index; actions[index].run(); sound('hold'); return end end
  if accept() then actions[self.helpSelection].run(); sound('hold') elseif pressed('back') then self.helpOpen=false; self.focusZone='primary' end
end

local function overlayListCount(self)
  if self.overlay=='setlist' then return #(room() and room().setlist or {}) end
  if self.overlay=='obs' then return 4 end
  if self.overlay=='history' then return #(BBT.history or {}) end
  return 0
end

local function overlayListSelection(self)
  if self.overlay=='setlist' then return self.setlistSelection end
  if self.overlay=='obs' then return self.streamSelection end
  return self.historySelection
end

local function setOverlayListSelection(self,value)
  if self.overlay=='setlist' then self.setlistSelection=value elseif self.overlay=='obs' then self.streamSelection=value else self.historySelection=value end
end

local function updateOverlay(self)
  local actions=overlayActions(self); local count=overlayListCount(self)
  if clicked(536,46,28,20) or pressed('back') then self.overlay=nil; self.focusZone='utility'; return end
  if pressed('menu_left') and count>0 then self.overlayFocus='list'; sound('click') end
  if pressed('menu_right') and #actions>0 then self.overlayFocus='actions'; sound('click') end
  if self.overlayFocus=='list' and count>0 then
    local selection=overlayListSelection(self)
    if self.overlay=='setlist' then
      local delta=pressed('menu_up') and -1 or (pressed('menu_down') and 1 or 0)
      if delta~=0 then selection,self.setlistOffset=Dashboard.scroll(selection,self.setlistOffset,count,delta,10); sound('click') end
    elseif self.overlay=='history' then
      local delta=pressed('menu_up') and -1 or (pressed('menu_down') and 1 or 0)
      if delta~=0 then selection,self.historyOffset=Dashboard.scroll(selection,self.historyOffset,count,delta,10); sound('click') end
    else
      if pressed('menu_up') then selection=(selection-2)%count+1; sound('click') end
      if pressed('menu_down') then selection=selection%count+1; sound('click') end
    end
    setOverlayListSelection(self,selection)
  elseif #actions>0 then
    if pressed('menu_up') then self.overlayActionSelection=(self.overlayActionSelection-2)%#actions+1; sound('click') end
    if pressed('menu_down') then self.overlayActionSelection=self.overlayActionSelection%#actions+1; sound('click') end
    self.overlayActionSelection=clamp(self.overlayActionSelection,1,#actions)
  end
  if self.overlay=='setlist' or self.overlay=='history' then
    local rows=math.min(10,count); local startY=self.overlay=='setlist' and 96 or 86; local step=self.overlay=='setlist' and 18 or 19
    local offset=self.overlay=='setlist' and (self.setlistOffset or 0) or (self.historyOffset or 0)
    for row=1,rows do if clicked(38,startY+(row-1)*step-3,330,17) then setOverlayListSelection(self,offset+row); self.overlayFocus='list' end end
  elseif self.overlay=='obs' then
    for index=1,4 do if clicked(38,81+(index-1)*47,330,40) then self.streamSelection=index; self.overlayFocus='list' end end
  end
  for index,item in ipairs(actions) do if clicked(390,94+(index-1)*24,172,20) then self.overlayActionSelection=index; self.overlayFocus='actions'; activate(item); return end end
  if accept() and self.overlayFocus=='actions' and #actions>0 then activate(actions[self.overlayActionSelection]) end
end

local function updateSideMenu(self)
  local actions=sideActions(self); if #actions==0 then self.sideMenu=nil; return end
  if pressed('menu_up') then self.sideSelection=(self.sideSelection-2)%#actions+1; sound('click') end
  if pressed('menu_down') then self.sideSelection=self.sideSelection%#actions+1; sound('click') end
  local start=self.sideMenu=='participant' and 181 or 112
  for index,item in ipairs(actions) do if clicked(360,start+(index-1)*24,216,20) then self.sideSelection=index; activate(item); return end end
  if accept() then activate(actions[self.sideSelection]) elseif pressed('back') then self.sideMenu=nil end
end

local function updateBase(self)
  local inRoom=room()~=nil; local secondary=secondaryActions(self); self.secondary=secondary
  if clicked(550,5,38,22) then self.helpOpen=true; self.helpSelection=1; return end
  for index,item in ipairs(UTILITIES) do
    local x=12+(index-1)*144
    if clicked(x,292,140,28) then self.utilitySelection=index; self.overlay=item.id; self.overlayActionSelection=1; self.overlayFocus=item.id=='settings' and 'actions' or 'list'; self.focusZone='utility'; return end
  end
  if inRoom then
    local participants=ensureRoster(self)
    for row=1,8 do
      local index=self.rosterOffset+row
      if participants[index] and clicked(19,122+(row-1)*19,340,17) then self.rosterSelection=index; self.focusZone='roster'; self.sideMenu='participant'; self.sideSelection=1; return end
    end
  end
  if clicked(388,inRoom and 151 or 110,188,36) then self.focusZone='primary'; local item=primaryAction(self); if item.enabled then runPrimary(self,item); sound('hold') else BBT.lastError=item.description end; return end
  local secondaryY=inRoom and 253 or 205
  for index,item in ipairs(secondary) do if clicked(388,secondaryY+(index-1)*23,188,19) then self.focusZone='secondary'; self.secondarySelection=index; activate(item); return end end

  if self.focusZone=='roster' and inRoom then
    local participants=ensureRoster(self)
    if pressed('menu_up') then self.rosterSelection,self.rosterOffset=Dashboard.scroll(self.rosterSelection,self.rosterOffset,#participants,-1,8); sound('click') end
    if pressed('menu_down') then self.rosterSelection,self.rosterOffset=Dashboard.scroll(self.rosterSelection,self.rosterOffset,#participants,1,8); sound('click') end
    if pressed('menu_right') then self.focusZone=Dashboard.nextFocus('roster','right',true,#secondary); sound('click') end
    if accept() and #participants>0 then self.sideMenu='participant'; self.sideSelection=1; sound('hold') end
  elseif self.focusZone=='primary' then
    if pressed('menu_left') and inRoom then self.focusZone=Dashboard.nextFocus('primary','left',inRoom,#secondary); sound('click') end
    if pressed('menu_up') then self.focusZone=Dashboard.nextFocus('primary','up',inRoom,#secondary); sound('click') end
    if pressed('menu_down') then self.focusZone=Dashboard.nextFocus('primary','down',inRoom,#secondary); sound('click') end
    if accept() then local item=primaryAction(self); if item.enabled then runPrimary(self,item); sound('hold') else BBT.lastError=item.description end end
  elseif self.focusZone=='secondary' then
    if pressed('menu_up') then self.secondarySelection=self.secondarySelection-1; if self.secondarySelection<1 then self.focusZone='primary'; self.secondarySelection=1 end; sound('click') end
    if pressed('menu_down') then self.secondarySelection=self.secondarySelection+1; if self.secondarySelection>#secondary then self.focusZone='utility'; self.secondarySelection=math.max(1,#secondary) end; sound('click') end
    if pressed('menu_left') and inRoom then self.focusZone='roster'; sound('click') end
    if accept() and secondary[self.secondarySelection] then activate(secondary[self.secondarySelection]) end
  elseif self.focusZone=='utility' then
    if pressed('menu_left') then self.utilitySelection=(self.utilitySelection-2)%#UTILITIES+1; sound('click') end
    if pressed('menu_right') then self.utilitySelection=self.utilitySelection%#UTILITIES+1; sound('click') end
    if pressed('menu_up') then self.focusZone=Dashboard.nextFocus('utility','up',inRoom,#secondary); sound('click') end
    if accept() then self.overlay=UTILITIES[self.utilitySelection].id; self.overlayActionSelection=1; self.overlayFocus=self.overlay=='settings' and 'actions' or 'list'; sound('hold') end
  else
    if accept() or pressed('menu_down') then self.helpOpen=true; self.helpSelection=1; sound('hold') end
    if pressed('menu_left') then self.focusZone=inRoom and 'roster' or 'primary' end
  end
  ensureRoster(self)
  if pressed('back') then requestConfirm(self,'EXIT ONLINE','Leave Online and stop the runtime, API, exports, and renderers?','EXIT',function() exitOnline(self) end) end
end

local function update(self)
  if self.confirm then updateConfirm(self); return end
  if self.formMode then updateForm(self); return end
  if self.helpOpen then updateHelp(self); return end
  if self.sideMenu then updateSideMenu(self); return end
  if self.overlay then updateOverlay(self); return end
  updateBase(self)
end

return function()
  local st=Gamestate:new('Online')
  st:setInit(function(self)
    em.clear({self.menuMusicManager}); mouse:disableGameplay(); shuv.resetPal()
    shuv.pal[2]={r=205,g=205,b=205}; shuv.pal[3]={r=255,g=52,b=50}
    shuv.pal[4]={r=224,g=227,b=0}; shuv.pal[5]={r=44,g=255,b=57}
    shuv.pal[6]={r=0,g=222,b=229}; shuv.pal[7]={r=63,g=38,b=255}; shuv.showBadColors=true
    BBT.lastError=nil
    self.focusZone='primary'; self.utilitySelection=1; self.secondarySelection=1
    self.rosterSelection=1; self.rosterOffset=0; self.setlistSelection=1; self.setlistOffset=0; self.streamSelection=1; self.historySelection=1; self.historyOffset=0
    self.overlayActionSelection=1; self.overlayFocus='list'; self.sideSelection=1; self.helpSelection=1; self.openForm=openForm
    BBT.startOnlineRuntime(); BBT.command('runtime.snapshot_request',{})
  end)
  function st:leave() if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(false) end end
  st:setUpdate(function(self,dt)
    if self.menuMusicManager then self.menuMusicManager:update(dt) end
    BBT.update(dt); if BBT.maybeLaunchScheduledChart() then return end; update(self)
  end)
  st:setFgDraw(function(self)
    drawBase(self)
    if self.overlay then drawOverlay(self) end
    if self.sideMenu then drawSideMenu(self) end
    if self.helpOpen then drawHelp(self) end
    if self.formMode then drawForm(self) end
    if self.confirm then drawConfirm(self) end
    setc(C.white)
  end)
  return st
end
