-- Protocol-v3 Online shell. The state is deliberately organized around one
-- workspace plus one optional modal; Back therefore has one predictable job
-- and selected participants survive filters, reordering, and reconnects.
local Dashboard = require('bbt.dashboard_model')
local Components = require('bbt.dashboard_components')
local hasUtf8,utf8 = pcall(require,'utf8')
if not hasUtf8 then utf8=nil end

local C = {
  black={0,0,0,1}, panel={0,0,0,1}, raised={1,0,0,1},
  -- Beatblock's shader accepts only its eight exact source colors. Source red
  -- maps to the menu's neutral gray slot; arbitrary RGB or alpha becomes the
  -- purple "bad color" pattern after the state transition disables bypass.
  white={1,1,1,1}, muted={1,0,0,1},
  red={0,0,1,1}, yellow={0,1,0,1}, green={1,1,0,1}, cyan={1,0,1,1}, blue={0,1,1,1},
}
local ui = Components.new(C)
local WORKSPACES = {
  {id='room',label='ROOM'}, {id='setlist',label='SETLIST'},
  {id='broadcast',label='BROADCAST'}, {id='history',label='HISTORY'},
  {id='settings',label='SETTINGS'}, {id='help',label='HELP'},
}
local FILTERS = {
  {id='all',label='ALL'}, {id='players',label='PLAYERS'},
  {id='spectators',label='SPECTATORS'}, {id='pending',label='PENDING'},
}
local STREAMS = {'A','B','C','D'}

local function pressed(name)
  if not maininput or not maininput.pressed then return false end
  local ok,value=pcall(maininput.pressed,maininput,name)
  return ok and value == true
end
local function accept() return pressed('accept') or pressed('select') end
local function currentRoom()
  local value=BBT.lastLobby
  return value and value.id~='offline' and value.lifecycle~='closed' and value or nil
end
local function isHost() return BBT.isOrganizer() end
local function hit(control)
  local mx,my=mouse and mouse.rx or -1,mouse and mouse.ry or -1
  return mx>=control.x and mx<=control.x+control.w and my>=control.y and my<=control.y+control.h
end
local function clicked(control) return mouse and mouse.pressed==1 and hit(control) end
local function context()
  return {
    room=currentRoom(), me=BBT.currentPlayer(), isHost=isHost(),
    chartVerified=BBT.chartVerified, runtimeReady=BBT.companionConnected,
    runtimeStarting=BBT.runtimeStarting,
  }
end
local function bounded(value,limit)
  value=tostring(value or '')
  if #value<=limit then return value end
  return value:sub(1,limit-3)..'...'
end

local function applyBeatblockPalette(showBadColors)
  if not shuv then return end
  shuv.resetPal()
  shuv.pal[2]={r=205,g=205,b=205}; shuv.pal[3]={r=255,g=52,b=50}
  shuv.pal[4]={r=224,g=227,b=0}; shuv.pal[5]={r=44,g=255,b=57}
  shuv.pal[6]={r=0,g=222,b=229}; shuv.pal[7]={r=63,g=38,b=255}
  -- Online draws exclusively with the eight source colors, while Beatblock's
  -- illustrated menus require non-indexed colors to pass through unchanged.
  shuv.showBadColors=showBadColors
end

local function applyBeatblockMenuFont()
  if love and love.graphics and fonts and fonts.digitalDisco then
    love.graphics.setFont(fonts.digitalDisco)
  end
end

local function leaveToMenu(self)
  local music=self.menuMusicManager
  if music then music:clearOnBeatHooks() end
  local previous=cs; cs=bs.load('Menu')
  if previous and previous.leave then previous:leave() end
  cs.menuMusicManager=music; cs:init()
end

local function register(self,id,x,y,w,h,run)
  local entry={id=id,x=x,y=y,w=w,h=h,run=run}
  self.controls[#self.controls+1]=entry
  return self.focusId==id
end

local function button(self,id,x,y,w,h,label,run,color,enabled)
  local focused=register(self,id,x,y,w,h,run)
  ui:button(id,x,y,w,h,label,focused,color,enabled)
end

local function chip(self,id,x,y,w,label,selected,run,color)
  register(self,id,x,y,w,22,run)
  ui:chip(id,x,y,w,label,selected,color)
end

local function setWorkspace(self,name)
  self.workspace=name
  self.focusId='nav_'..name
  self.modal=nil
end

local function selected(self)
  local participant,list=Dashboard.selectedParticipant(context(),self.rosterFilter,self.selectedSessionId)
  if participant then self.selectedSessionId=participant.sessionId end
  return participant,list
end

local function openConfirm(self,title,message,label,run)
  self.modal={kind='confirm',title=title,message=message,label=label or 'CONFIRM',run=run,returnFocus=self.focusId}
  self.focusId='modal_cancel'
end

local function openForm(self,mode,spectator)
  local settings=BBT.settings or {}
  self.modal={
    kind='form', mode=mode, spectator=spectator==true,
    title=mode=='host' and 'HOST ROOM' or (spectator and 'JOIN AS SPECTATOR' or 'JOIN AS PLAYER'),
    values={
      displayName=tostring(BBT.context and BBT.context.playerName or 'Player'),
      name='Beatblock Room', address=tostring(settings.hostAddress or '127.0.0.1'),
      port=tostring(settings.hostPort or 32145), password='',
    },
    fields=mode=='host' and {'displayName','name','port','password'} or {'displayName','address','port','password'},
    index=1,
    returnFocus=self.focusId,
  }
  self.focusId='form_'..self.modal.fields[1]
  if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(true) end
end

local function closeModal(self)
  local returnFocus=self.modal and self.modal.returnFocus
  self.modal=nil
  self.focusId=returnFocus or 'session_primary'
  if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(false) end
end

local function submitForm(self)
  local modal=self.modal
  if not modal or modal.kind~='form' then return end
  local values=modal.values
  local function missing(value) return tostring(value or ''):match('^%s*$')~=nil end
  local port=tonumber(values.port)
  if missing(values.displayName) then modal.error='DISPLAY NAME IS REQUIRED'; return end
  if modal.mode=='host' and missing(values.name) then modal.error='ROOM NAME IS REQUIRED'; return end
  if modal.mode~='host' and missing(values.address) then modal.error='HOST ADDRESS IS REQUIRED'; return end
  if not port or port<1 or port>65535 or port%1~=0 then modal.error='UDP PORT MUST BE 1-65535'; return end
  if missing(values.password) then modal.error='PASSWORD IS REQUIRED'; return end
  modal.error=nil
  if modal.mode=='host' then
    BBT.command('room.host_request',{
      displayName=values.displayName,name=values.name,password=values.password,
      port=port,hostApproval=true,allowChartTransfers=true,
    })
  else
    BBT.command('room.join_request',{
      displayName=values.displayName,address=values.address..':'..values.port,
      password=values.password,spectator=modal.spectator,
    })
  end
  closeModal(self)
end

local function header(self)
  ui:text('BBT  /  ONLINE',12,7,170,'left','white')
  local status=BBT.companionConnected and 'ONLINE  /  PROTOCOL V3' or (BBT.runtimeStarting and 'STARTING ONLINE' or 'RUNTIME OFFLINE')
  ui:text(status,300,7,288,'right',BBT.companionConnected and 'green' or 'yellow')
  local room=currentRoom()
  ui:panel(12,27,576,44)
  if room then
    local chart=room.chart
    ui:text(room.name or 'ONLINE SESSION',22,34,220,'left','muted')
    ui:text(chart and (chart.songName or chart.packageName) or 'NO CHART SELECTED',22,49,300,'left',chart and 'white' or 'yellow')
    if chart then ui:text((chart.variant or '')..(chart.official and '  /  OFFICIAL' or '  /  CUSTOM'),325,34,125,'right','muted') end
  else
    ui:text('ONLINE SESSION',22,34,220,'left','muted')
    ui:text(BBT.companionConnected and 'READY TO CONNECT' or 'LOCAL RUNTIME REQUIRED',22,49,300,'left',BBT.companionConnected and 'green' or 'yellow')
  end
  local primary=Dashboard.primary(context())
  button(self,'session_primary',454,36,124,26,primary.label,function() runPrimary(self,primary) end,primary.tone,primary.enabled)
end

function runPrimary(self,item)
  if item.id=='host_room' then openForm(self,'host')
  elseif item.id=='open_installer' then BBT.openInstaller()
  elseif item.id=='select_chart' then setWorkspace(self,'setlist')
  elseif item.id=='locate_chart' then
    local room=currentRoom()
    if room and room.chart then
      if room.chart.official then BBT.openOfficialSelect('verify') else BBT.openChartSelect('verify') end
    end
  elseif item.id=='ready' then BBT.command('room.ready_request',{ready=true})
  elseif item.id=='start_race' then BBT.command('room.start_request',{force=false})
  elseif item.id=='advance_set' then BBT.command('setlist.advance',{})
  elseif item.id=='view_results' then setWorkspace(self,'room') end
end

local function participantActionButtons(self,target,x,y,w)
  local room=currentRoom()
  if not target or not room then return y end
  if isHost() and target.sessionId~=room.hostSessionId then
    if target.admitted~=true then
      button(self,'participant_approve',x,y,w,24,'APPROVE',function()
        BBT.command('room.admission_set',{sessionId=target.sessionId,admit=true,role=target.role})
      end,'green',room.lifecycle~='playing' and room.lifecycle~='countdown')
      y=y+29
      button(self,'participant_reject',x,y,w,24,'REJECT',function()
        openConfirm(self,'REJECT REQUEST','Reject '..bounded(target.displayName,32)..' from this room?','REJECT',function()
          BBT.command('room.admission_set',{sessionId=target.sessionId,admit=false,role=target.role})
        end)
      end,'red')
      return y+29
    end
    button(self,'participant_role',x,y,w,24,target.role=='spectator' and 'MAKE PLAYER' or 'MAKE SPECTATOR',function()
      BBT.command('room.role_set',{sessionId=target.sessionId,role=target.role=='spectator' and 'player' or 'spectator'})
    end,'yellow',room.lifecycle~='playing' and room.lifecycle~='countdown')
    y=y+29
    if target.role=='spectator' then
      button(self,'participant_commentator',x,y,w,24,target.commentatorAccess and 'REVOKE COMMENTATOR' or 'GRANT COMMENTATOR',function()
        BBT.command('room.commentator_set',{sessionId=target.sessionId,enabled=not target.commentatorAccess})
      end,target.commentatorAccess and 'yellow' or 'cyan')
      y=y+29
    end
    button(self,'participant_remove',x,y,w,24,'REMOVE',function()
      openConfirm(self,'REMOVE PARTICIPANT','Remove '..bounded(target.displayName,32)..' from the room?','REMOVE',function()
        BBT.command('room.kick',{sessionId=target.sessionId})
      end)
    end,'red',room.lifecycle~='playing' and room.lifecycle~='countdown')
  elseif target.sessionId==(BBT.context and BBT.context.sessionId) then
    local transfer=BBT.chartTransfer
    if transfer and (transfer.state=='offer' or transfer.state=='consent') then
      button(self,'participant_transfer_accept',x,y,w,24,'ACCEPT TRANSFER',function()
        local run=function()
          BBT.command('chart.transfer_decision',{
            requestId=transfer.requestId,accept=true,trustRoom=false,
            executableContentConfirmed=transfer.containsExecutableContent==true,
          })
        end
        if transfer.containsExecutableContent then
          openConfirm(self,'SCRIPT CONTENT','This package contains script or executable content. Only accept it if you trust this room host.','ACCEPT',run)
        else run() end
      end,'green')
      y=y+29
      button(self,'participant_transfer_trust',x,y,w,24,'TRUST THIS ROOM',function()
        BBT.command('chart.transfer_decision',{
          requestId=transfer.requestId,accept=true,trustRoom=true,
          executableContentConfirmed=false,
        })
      end,'cyan',not transfer.containsExecutableContent)
      y=y+29
    elseif target.role~='spectator' and not target.verified and room.chart then
      button(self,'participant_locate',x,y,w,24,'SELECT LOCAL CHART',function()
        if room.chart.official then BBT.openOfficialSelect('verify') else BBT.openChartSelect('verify') end
      end,'cyan')
      y=y+29
      button(self,'participant_transfer',x,y,w,24,'REQUEST HOST TRANSFER',function()
        BBT.command('chart.transfer_request',{chartHash=room.chart.hash})
      end,'yellow',not room.chart.official and room.chart.transferMode=='host_transfer')
      y=y+29
    elseif target.ready and (room.lifecycle=='forming' or room.lifecycle=='chart_locked' or room.lifecycle=='ready') then
      button(self,'participant_unready',x,y,w,24,'UNREADY',function() BBT.command('room.ready_request',{ready=false}) end,'yellow')
      y=y+29
    end
    local action=isHost() and 'CLOSE ROOM' or 'LEAVE ROOM'
    button(self,'participant_leave_room',x,y,w,24,action,function()
      openConfirm(self,action,(isHost() and 'Close this room for every participant?' or 'Leave this room and keep Online available?'),action,function()
        BBT.command(isHost() and 'room.close_request' or 'room.leave_request',{})
      end)
    end,'red',room.lifecycle~='playing' and room.lifecycle~='countdown')
  end
end

local function drawRoster(self,results)
  local room=currentRoom()
  ui:panel(12,78,360,225,results and 'CURRENT RESULTS' or 'PARTICIPANTS')
  local x=20
  for _,filter in ipairs(FILTERS) do
    local width=filter.id=='spectators' and 87 or 65
    chip(self,'filter_'..filter.id,x,104,width,filter.label,self.rosterFilter==filter.id,function()
      self.rosterFilter=filter.id
      local selectedPlayer=Dashboard.selectedParticipant(context(),filter.id,self.selectedSessionId)
      self.selectedSessionId=selectedPlayer and selectedPlayer.sessionId or nil
    end,filter.id=='pending' and 'yellow' or 'cyan')
    x=x+width+5
  end
  local target,list=selected(self)
  local lifecycle=room.lifecycle
  local showScore=lifecycle=='playing' or lifecycle=='results' or lifecycle=='set_complete'
  ui:text('NAME',20,132,142,'left','muted')
  if showScore then
    ui:text('RANK',205,132,48,'right','muted'); ui:text('ACCURACY',267,132,92,'right','muted')
  else ui:text('STATE',238,132,121,'right','muted') end
  for index,participant in ipairs(list) do
    if index>7 then break end
    local rowY=146+(index-1)*21
    local focused=self.selectedSessionId==participant.sessionId
    register(self,'participant_'..participant.sessionId,18,rowY,346,20,function()
      self.selectedSessionId=participant.sessionId
    end)
    if focused then ui:color('raised'); love.graphics.rectangle('fill',18,rowY,346,20,2,2) end
    local role=participant.role=='spectator' and (participant.commentatorAccess and '[C]' or '[S]') or '[P]'
    ui:text(role..' '..participant.displayName,23,rowY+4,174,'left',focused and 'black' or 'white')
    if showScore then
      local score=Dashboard.score(participant,lifecycle)
      ui:text(score.rank or '—',185,rowY+4,72,'right',focused and 'black' or score.tone or 'white')
      ui:text(score.accuracy or '—',265,rowY+4,94,'right',focused and 'black' or score.tone or 'white')
    else
      local label,color=Dashboard.participantStatus(participant)
      ui:text(label,226,rowY+4,133,'right',focused and 'black' or color)
    end
  end
  if #list==0 then ui:text('NO PARTICIPANTS IN THIS FILTER',30,178,324,'center','muted') end
  return target
end

local function drawInspector(self,target)
  local room=currentRoom()
  ui:panel(379,78,209,225,'PARTICIPANT')
  if not target then
    ui:wrapped('Select a participant to inspect their role, connection, chart verification, and host actions.',391,110,185,6,'muted')
    return
  end
  ui:text(target.displayName,391,106,185,'left','cyan')
  local role=target.role=='spectator' and (target.commentatorAccess and 'COMMENTATOR' or 'SPECTATOR') or 'PLAYER'
  local labels={
    {'ROLE',role}, {'CONNECTION',target.connected==false and 'OFFLINE' or 'CONNECTED'},
    {'CHART',target.role=='spectator' and 'NOT REQUIRED' or (target.verified and 'VERIFIED' or 'MISMATCH')},
    {'RUN',target.validity=='dnf' and 'DNF' or target.validity=='invalid' and 'INVALID' or target.ready and 'READY' or 'WAITING'},
  }
  if room and (room.lifecycle=='results' or room.lifecycle=='set_complete') and target.role~='spectator' then
    labels[#labels+1]={'SET TOTAL',target.setTotal and string.format('%.2f',target.setTotal) or '—'}
  end
  for index,item in ipairs(labels) do
    local y=126+(index-1)*17
    ui:text(item[1],391,y,78,'left','muted')
    ui:text(item[2],469,y,107,'right',(item[2]=='MISMATCH' or item[2]=='INVALID' or item[2]=='DNF') and 'red' or 'white')
  end
  local transfer=BBT.chartTransfer
  if transfer and target.sessionId==(BBT.context and BBT.context.sessionId) then
    local copy=transfer.state=='progress' and ('TRANSFER '..tostring(transfer.percent or 0)..'%')
      or transfer.state=='offer' and 'TRANSFER OFFER AVAILABLE'
      or transfer.state=='consent' and 'CONSENT REQUIRED'
      or nil
    if copy then ui:text(copy,391,193,185,'left',transfer.state=='progress' and 'cyan' or 'yellow') end
  end
  participantActionButtons(self,target,391,199,185)
end

local function drawConnect(self)
  ui:panel(12,78,576,225,'CONNECT')
  ui:text('CHOOSE HOW YOU JOIN',24,108,552,'center','cyan')
  ui:wrapped('Create a direct-IP room from the session action above, or join an existing room below.',50,129,500,2,'muted')

  ui:text('PLAYER',32,161,250,'left','white')
  ui:wrapped('Compete, verify the locked chart, then ready up.',32,178,250,2,'muted')
  ui:text('SPECTATOR',318,161,250,'left','white')
  ui:wrapped('Watch rankings without scoring. Commentator is host-granted.',318,178,250,2,'muted')

  button(self,'connect_join',32,211,250,32,'JOIN AS PLAYER',function() openForm(self,'join',false) end,'cyan',BBT.companionConnected)
  button(self,'connect_spectate',318,211,250,32,'JOIN AS SPECTATOR',function() openForm(self,'join',true) end,'white',BBT.companionConnected)
  button(self,'connect_exit',418,256,150,27,'EXIT ONLINE',function()
    openConfirm(self,'EXIT ONLINE','Stop the Online runtime and return to the main menu?','EXIT',function() BBT.exitOnline(); leaveToMenu(self) end)
  end,'red')
  local problem=BBT.lastError
  if not problem and not BBT.companionConnected then
    problem=BBT.runtimeLaunchStatus or 'The local runtime is unavailable.'
  end
  if problem then
    ui:wrapped(bounded(problem,180),32,255,368,3,'red')
  else
    ui:text('HOSTING? USE THE SESSION ACTION ABOVE.',32,263,368,'left','muted')
  end
end

local function drawRoom(self)
  local room=currentRoom()
  if not room then drawConnect(self); return end
  local results=room.lifecycle=='results' or room.lifecycle=='set_complete'
  local target=drawRoster(self,results)
  drawInspector(self,target)
end

local function drawSetlist(self)
  local room=currentRoom()
  ui:panel(12,78,364,225,'SETLIST')
  ui:panel(383,78,205,225,'ACTIONS')
  if not room then ui:wrapped('Join or host a room before building a setlist.',24,111,340,3,'muted'); return end
  local entries=room.setlist or {}
  for index,entry in ipairs(entries) do
    if index>8 then break end
    local y=105+(index-1)*22
    local active=room.currentSetlistIndex==index-1
    if active then ui:color('raised'); love.graphics.rectangle('fill',20,y,348,20,2,2) end
    ui:text(tostring(index)..'.  '..(entry.chart.songName or entry.chart.packageName or 'Chart'),25,y+4,249,'left',active and 'black' or 'white')
    ui:text(entry.chart.variant or '',281,y+4,78,'right',active and 'black' or 'muted')
  end
  if #entries==0 then ui:text('NO CHARTS IN THE SET',28,130,332,'center','muted') end
  local canEdit=isHost() and room.lifecycle~='playing' and room.lifecycle~='countdown'
  button(self,'setlist_official',395,105,181,25,'SELECT OFFICIAL',function() BBT.openOfficialSelect('host') end,'cyan',canEdit)
  button(self,'setlist_custom',395,136,181,25,'SELECT CUSTOM',function() BBT.openChartSelect('host') end,'cyan',canEdit)
  button(self,'setlist_add_official',395,177,181,25,'ADD OFFICIAL',function() BBT.openOfficialSelect('setlist') end,'green',canEdit)
  button(self,'setlist_add_custom',395,208,181,25,'ADD CUSTOM',function() BBT.openChartSelect('setlist') end,'green',canEdit)
  ui:wrapped(isHost() and 'The host controls chart order. Custom locked packages can use host transfer.' or 'This setlist is controlled by the host.',395,246,181,4,'muted')
end

local function rendererSlot(id)
  for _,slot in ipairs(BBT.renderers or {}) do if slot.id==id then return slot end end
  return {id=id,active=false,featured=id=='A',mode='full',width=1280,height=720,fps=60,delayMs=500}
end

local function planSlot(id)
  local plan=BBT.broadcastPlan or (BBT.runtimeSnapshot and BBT.runtimeSnapshot.broadcastPlan)
  for _,slot in ipairs(plan and plan.slots or {}) do if slot.id==id then return slot end end
  return rendererSlot(id)
end

local function drawBroadcast(self)
  local allowed,authority=Dashboard.canBroadcast(context())
  ui:panel(12,78,576,225,'BROADCAST')
  if not allowed then
    ui:text('BROADCAST IS NOT AVAILABLE',48,121,504,'center','yellow')
    ui:wrapped('Ordinary Spectators can follow the room and rankings. A host may grant Commentator access from the participant inspector.',95,151,410,5,'muted')
    return
  end
  local target=selected(self)
  local rendererEditable=room.lifecycle~='playing' and room.lifecycle~='countdown'
  ui:text(authority=='host' and 'HOST PLAN' or 'HOST PLAN  /  READ ONLY',24,105,270,'left','cyan')
  ui:text(target and ('CANDIDATE: '..target.displayName) or 'CANDIDATE: SELECT A PLAYER',306,105,270,'right','muted')
  for index,id in ipairs(STREAMS) do
    local slot=authority=='host' and rendererSlot(id) or planSlot(id)
    local x=20+(index-1)*140
    ui:color(slot.active and (slot.featured and 'cyan' or 'raised') or 'panel')
    love.graphics.rectangle('fill',x,126,132,104,3,3)
    ui:color('raised'); love.graphics.rectangle('line',x+.5,126.5,131,103,3,3)
    ui:text('STREAM '..id,x+7,134,118,'left',slot.active and 'black' or 'white')
    ui:text(slot.participantName or slot.participant_name or 'UNASSIGNED',x+7,153,118,'left',slot.active and 'black' or 'muted')
    local health=slot.lastError and 'ERROR' or slot.healthy and 'HEALTHY' or slot.active and 'STARTING' or 'STOPPED'
    ui:text(health,x+7,173,118,'left',slot.lastError and 'red' or slot.healthy and 'green' or 'muted')
    if authority=='host' then
      button(self,'broadcast_assign_'..id,x+7,196,56,25,slot.active and 'STOP' or 'ASSIGN',function()
        if slot.active then BBT.command('renderer.stop',{slot=id})
        elseif target and target.role~='spectator' then
          BBT.command('renderer.configure',{slot=id,participantId=target.sessionId,participantName=target.displayName,mode='full',width=1280,height=720,fps=60,delayMs=500,featured=slot.featured})
        end
      end,slot.active and 'yellow' or 'cyan',slot.active or (rendererEditable and target and target.role~='spectator'))
      button(self,'broadcast_feature_'..id,x+68,196,57,25,'FEATURE',function()
        BBT.command('renderer.configure',{slot=id,participantId=slot.participantId,participantName=slot.participantName,mode=slot.mode,width=slot.width,height=slot.height,fps=slot.fps,delayMs=slot.delayMs,featured=true})
      end,'green',rendererEditable and slot.active and not slot.featured)
    end
  end
  if authority=='commentator' then
    local enabled=BBT.mirrorEnabled or (BBT.runtimeSnapshot and BBT.runtimeSnapshot.mirrorEnabled)
    ui:text('THIS PC',24,244,100,'left','white')
    ui:text(enabled and 'LOCAL MIRROR ENABLED' or 'LOCAL MIRROR DISABLED',112,244,250,'left',enabled and 'green' or 'yellow')
    button(self,'broadcast_mirror',398,238,178,27,enabled and 'DISABLE MIRROR' or 'ENABLE MIRROR',function()
      if enabled then BBT.command('broadcast.mirror_set',{enabled=false})
      else
        openConfirm(self,'ENABLE LOCAL MIRROR','This may start up to four hidden renderer processes and increase CPU/GPU use. Continue?','ENABLE',function()
          BBT.command('broadcast.mirror_set',{enabled=true})
        end)
      end
    end,enabled and 'yellow' or 'cyan')
  else
    button(self,'broadcast_advanced',398,238,178,27,self.broadcastAdvanced and 'HIDE ADVANCED' or 'ADVANCED',function() self.broadcastAdvanced=not self.broadcastAdvanced end,'white')
  end
  local detail
  for _,slot in ipairs(BBT.renderers or {}) do if slot.lastError then detail=slot.lastError break end end
  if detail then
    ui:text('RENDERER: '..bounded(detail,48),24,274,465,'left','red')
    button(self,'broadcast_details',496,270,80,24,'DETAILS',function()
      self.modal={kind='details',title='RENDERER DETAILS',message=detail,returnFocus=self.focusId}
    end,'white')
  elseif self.broadcastAdvanced and authority=='host' then
    ui:text('MODE FULL  /  1280x720  /  60 FPS  /  500 MS  /  FEATURED AUDIO ONLY',24,276,552,'left','muted')
  else
    ui:text('Featured video, text exports, and audio follow the same delayed clock.',24,276,552,'left','muted')
  end
end

local function drawHistory(self)
  ui:panel(12,78,382,225,'MATCH HISTORY')
  ui:panel(401,78,187,225,'ACTIONS')
  local history=BBT.history or {}
  for index,item in ipairs(history) do
    if index>8 then break end
    local y=106+(index-1)*22
    ui:text(item.name or item.roomName or 'Beatblock Room',24,y,235,'left','white')
    ui:text(item.lifecycle or item.status or 'CLOSED',270,y,108,'right','muted')
  end
  if #history==0 then ui:text('NO SAVED MATCHES',30,139,346,'center','muted') end
  button(self,'history_refresh',413,106,163,26,'REFRESH',function() BBT.command('history.list',{}) end,'cyan')
  button(self,'history_prune',413,140,163,26,'PRUNE EVENTS',function()
    openConfirm(self,'PRUNE RAW EVENTS','Remove raw event journals older than 30 days? Match summaries remain available.','PRUNE',function()
      BBT.command('history.prune',{days=30})
    end)
  end,'yellow')
  ui:wrapped('History is the archive. Current Results remain in the Room workspace.',413,185,163,5,'muted')
end

local function drawSettings(self)
  ui:panel(12,78,360,225,'SETTINGS')
  ui:panel(379,78,209,225,'RUNTIME')
  local settings=BBT.settings or {}
  local rows={
    {'GAMEPLAY HUD',settings.hudEnabled==false and 'OFF' or 'ON'},
    {'CHART TRANSFERS','HOST DEFAULT: ON'},
    {'TRANSFER CACHE',tostring((BBT.runtimeSnapshot and BBT.runtimeSnapshot.chartCacheSizeLabel) or '0 MB / 2 GB')},
    {'PROTOCOL','V3 ONLY'},
  }
  for index,row in ipairs(rows) do
    local y=108+(index-1)*26
    ui:text(row[1],24,y,145,'left','muted'); ui:text(row[2],169,y,189,'right','white')
  end
  button(self,'settings_hud',24,218,160,25,'TOGGLE HUD',function()
    BBT.command('settings.update',{hudEnabled=not (settings.hudEnabled~=false)})
  end,'cyan')
  button(self,'settings_clear_cache',194,218,164,25,'CLEAR CACHE',function()
    openConfirm(self,'CLEAR TRANSFER CACHE','Remove inactive BBT-managed chart packages? The active chart is protected.','CLEAR',function()
      BBT.command('chart.cache_clear',{})
    end)
  end,'yellow')
  ui:text('CONNECTION',391,107,185,'left','muted')
  ui:text((BBT.runtimeSnapshot and BBT.runtimeSnapshot.connection) or 'LOCAL',391,125,185,'left','green')
  ui:text('JOIN ADDRESS',391,153,185,'left','muted')
  ui:text((BBT.runtimeSnapshot and BBT.runtimeSnapshot.joinAddress) or '—',391,171,185,'left','white')
  button(self,'settings_logs',391,211,185,25,'OPEN LOGS',function() BBT.command('paths.open_logs',{}) end,'white')
  button(self,'settings_exports',391,242,185,25,'OPEN EXPORTS',function() BBT.command('paths.open_exports',{}) end,'white')
  button(self,'settings_diagnostics',391,273,185,25,'REFRESH DIAGNOSTICS',function() BBT.command('diagnostics.get',{}) end,'cyan')
end

local function drawHelp(self)
  ui:panel(12,78,576,225,'HELP')
  ui:text('ROOM ROLES',24,106,160,'left','cyan')
  ui:wrapped('Player competes. Spectator watches. Commentator is a host-granted Spectator permission that can mirror the Host Plan to this PC.',24,124,262,6,'muted')
  ui:text('CHARTS & TRANSFER',310,106,250,'left','cyan')
  ui:wrapped('Online searches local charts first. Custom packages may be transferred with consent; scripts always need separate confirmation. Cache entries are managed by BBT.',310,124,254,7,'muted')
  ui:text('CONTROLS',24,216,160,'left','cyan')
  ui:wrapped('Arrows navigate  •  Enter selects  •  Esc returns one layer  •  Mouse uses the same focus.',24,234,262,4,'muted')
  ui:text('TROUBLESHOOTING',310,216,250,'left','cyan')
  ui:wrapped('Open Logs for full runtime errors. Broadcast error summaries stay bounded so controls never disappear.',310,234,254,4,'muted')
end

local function drawNavigation(self)
  local allowed=Dashboard.canBroadcast(context())
  local visible={}
  for _,workspace in ipairs(WORKSPACES) do
    if workspace.id~='broadcast' or allowed then visible[#visible+1]=workspace end
  end
  local gap=4
  local width=math.floor((576-gap*(#visible-1))/#visible)
  local x=12
  for _,workspace in ipairs(visible) do
    chip(self,'nav_'..workspace.id,x,306,width,workspace.label,self.workspace==workspace.id,function() setWorkspace(self,workspace.id) end,'cyan')
    x=x+width+gap
  end
  local hint=self.modal and 'ESC: CLOSE  /  ENTER: SELECT' or 'ARROWS: NAVIGATE  /  ENTER: SELECT  /  ESC: BACK'
  ui:text(hint,12,333,576,'center','muted')
end

local function drawModal(self)
  if not self.modal then return end
  ui:veil()
  local modal=self.modal
  if modal.kind=='form' then
    ui:panel(118,60,364,240,modal.title)
    for index,key in ipairs(modal.fields) do
      local y=96+(index-1)*38
      local labels={displayName='DISPLAY NAME',name='ROOM NAME',address='HOST ADDRESS',port='UDP PORT',password='PASSWORD'}
      ui:text(labels[key],136,y,124,'left','muted')
      local value=key=='password' and string.rep('*',#modal.values[key]) or modal.values[key]
      button(self,'form_'..key,261,y-5,203,27,value,function() modal.index=index; self.focusId='form_'..key end,'white')
    end
    button(self,'form_submit',261,252,98,27,modal.mode=='host' and 'CREATE' or 'JOIN',function() submitForm(self) end,'green')
    button(self,'form_cancel',366,252,98,27,'CANCEL',function() closeModal(self) end,'white')
    if modal.error then ui:text(modal.error,136,282,328,'center','red') end
  else
    ui:panel(126,99,348,162,modal.title)
    ui:wrapped(modal.message,146,133,308,5,modal.kind=='details' and 'red' or 'white')
    if modal.kind=='confirm' then
      button(self,'modal_confirm',146,218,143,27,modal.label,function() local run=modal.run; closeModal(self); run() end,'red')
      button(self,'modal_cancel',311,218,143,27,'CANCEL',function() closeModal(self) end,'white')
    else
      button(self,'modal_cancel',311,218,143,27,'CLOSE',function() closeModal(self) end,'white')
    end
  end
end

local function draw(self)
  -- Native states share one global LÖVE font. Reassert Online's font every
  -- frame so a popup or selector transition cannot leave stale metrics behind.
  applyBeatblockMenuFont()
  ui:begin(); self.controls={}
  header(self)
  if self.workspace=='setlist' then drawSetlist(self)
  elseif self.workspace=='broadcast' then drawBroadcast(self)
  elseif self.workspace=='history' then drawHistory(self)
  elseif self.workspace=='settings' then drawSettings(self)
  elseif self.workspace=='help' then drawHelp(self)
  else drawRoom(self) end
  drawNavigation(self)
  if self.modal then self.controls={} end
  drawModal(self)
  local focused=false
  for _,control in ipairs(self.controls) do if control.id==self.focusId then focused=true end end
  if not focused and self.controls[1] then self.focusId=self.controls[1].id; focused=true end
  if not focused then ui.audit.issues[#ui.audit.issues+1]='focus_outside_active_workspace' end
  for left=1,#self.controls do
    for right=left+1,#self.controls do
      local a,b=self.controls[left],self.controls[right]
      if a.x < b.x+b.w and b.x < a.x+a.w and a.y < b.y+b.h and b.y < a.y+a.h then
        ui.audit.issues[#ui.audit.issues+1]='control_overlap:'..a.id..':'..b.id
      end
    end
  end
  BBT.layoutAudit=ui.audit
end

local function focusIndex(self)
  for index,control in ipairs(self.controls or {}) do if control.id==self.focusId then return index end end
  return 1
end

local function update(self)
  local controls=self.controls or {}
  if #controls==0 then return end
  for _,control in ipairs(controls) do
    if hit(control) then self.focusId=control.id end
    if clicked(control) and control.run then control.run(); return end
  end
  local delta=(pressed('menu_down') or pressed('menu_right')) and 1 or (pressed('menu_up') or pressed('menu_left')) and -1 or 0
  if delta~=0 then
    local index=((focusIndex(self)-1+delta)%#controls)+1
    self.focusId=controls[index].id
    return
  end
  if accept() then
    local control=controls[focusIndex(self)]
    if control and control.run then control.run() end
    return
  end
  if pressed('back') then
    if self.modal then
      closeModal(self)
    elseif self.workspace~='room' then setWorkspace(self,'room')
    else
      openConfirm(self,'EXIT ONLINE','Stop the Online runtime and return to the main menu?','EXIT',function() BBT.exitOnline(); leaveToMenu(self) end)
    end
  end
end

local function editText(self,text)
  local modal=self.modal
  if not modal or modal.kind~='form' then return end
  local key=modal.fields[modal.index]
  if #modal.values[key] < 64 and text:match('[%g ]') then
    modal.values[key]=modal.values[key]..text
    modal.error=nil
  end
end

local function removeLastCharacter(value)
  value=tostring(value or '')
  if value=='' then return value end
  -- LÖVE textinput values are UTF-8. Use a codepoint boundary when available
  -- so Backspace removes one visible character instead of corrupting its bytes.
  if utf8 and utf8.offset then
    local ok,index=pcall(utf8.offset,value,-1)
    if ok and index then return value:sub(1,index-1) end
  end
  return value:sub(1,-2)
end

local function editKey(self,key)
  local modal=self.modal
  if not modal or modal.kind~='form' or (key~='backspace' and key~='delete') then return end
  local field=modal.fields[modal.index]
  modal.values[field]=removeLastCharacter(modal.values[field])
  modal.error=nil
end

return function()
  local st=Gamestate:new('Online')
  function st:openForm(mode,spectator) openForm(self,mode,spectator) end
  function st:submitForm() submitForm(self) end
  st:setInit(function(self,options)
    options=options or {}
    applyBeatblockPalette(false)
    applyBeatblockMenuFont()
    self.workspace=options.workspace or 'room'; self.rosterFilter='all'; self.selectedSessionId=nil
    self.focusId=self.workspace=='room' and 'session_primary' or 'nav_'..self.workspace
    self.controls={}; self.broadcastAdvanced=false; self.modal=nil
    -- Online is a complete state, not a menu modal. Suppress the entity
    -- manager retained from Menu and clear those entities before Song Select
    -- can inherit them on the next transition.
    self.holdEntityDraw=true
    if em and em.clear then em.clear({self.menuMusicManager}) end
    if mouse and mouse.disableGameplay then mouse:disableGameplay() end
    self.previousTextInput=love.textinput
    self.onlineTextInput=function(text)
      if self.modal and self.modal.kind=='form' then editText(self,text)
      elseif self.previousTextInput then self.previousTextInput(text) end
    end
    love.textinput=self.onlineTextInput
    -- Beatblock's native key callback does not edit custom text fields. Chain
    -- it so engine/ImGui behavior survives while forms gain deletion support.
    self.previousKeyPressed=love.keypressed
    self.onlineKeyPressed=function(key,scancode,isRepeat)
      if self.previousKeyPressed then self.previousKeyPressed(key,scancode,isRepeat) end
      editKey(self,key)
    end
    love.keypressed=self.onlineKeyPressed
    BBT.startOnlineRuntime()
    if not BBT.pendingRequestId then BBT.command('runtime.snapshot_request',{}) end
  end)
  function st:leave()
    if love.textinput==self.onlineTextInput then love.textinput=self.previousTextInput end
    if love.keypressed==self.onlineKeyPressed then love.keypressed=self.previousKeyPressed end
    if love.keyboard and love.keyboard.setTextInput then love.keyboard.setTextInput(false) end
    -- Restore the native menu shader before any destination state draws. Some
    -- transitions reuse an already-loaded Menu state, so relying on Menu:init
    -- alone leaves its full-color artwork rendered as the bad-color pattern.
    applyBeatblockPalette(true)
    applyBeatblockMenuFont()
  end
  st:setUpdate(function(self,dt)
    if self.menuMusicManager then self.menuMusicManager:update(dt) end
    BBT.update(dt)
    if BBT.maybeLaunchScheduledChart() then return end
    update(self)
  end)
  st:setBgDraw(function(self)
    ui:color('black')
    love.graphics.rectangle('fill',0,0,project.res.x,project.res.y)
  end)
  st:setFgDraw(function(self) draw(self); ui:color('white') end)
  return st
end
