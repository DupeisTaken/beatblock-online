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
-- Chip widths are sized for the game's real 16pt menu font, not for an
-- abbreviation that only fits a smaller QA font.
local FILTERS = {
  {id='all',label='ALL',width=54}, {id='players',label='PLAYERS',width=76},
  {id='spectators',label='SPECTATORS',width=100}, {id='pending',label='PENDING',width=74},
}
local STREAMS = {'A','B','C','D'}
local ROSTER_PAGE_SIZE = 6
-- Inspector action stack pitch: a 24px control plus a 2px gap. Four stacked
-- actions plus five detail rows still end on the panel floor at 16pt.
local ACTION_PITCH = 26
local DEFAULT_MODIFIERS={
  rate=1.0,vfx='full',taps='default',sides='default',barelies='default',restartOn='none',
}
local MODIFIER_CHOICES={
  vfx={{'full','FULL'},{'decreased','DECREASED'},{'none','NONE'}},
  taps={{'default','DEFAULT'},{'lenient','LENIENT'},{'strict','STRICT'},{'auto','AUTO'}},
  sides={{'default','DEFAULT'},{'lenient','LENIENT'},{'auto','AUTO'}},
  barelies={{'default','DEFAULT'},{'lenient','LENIENT'},{'strict','STRICT'}},
  restartOn={{'none','NONE'},{'miss','MISS'},{'barely','BARELY'}},
}

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
  local finalByte=math.max(0,limit-3)
  -- Do not split a multibyte name or diagnostic at the byte budget. LÖVE's
  -- text renderer rejects malformed UTF-8 instead of displaying a replacement.
  while finalByte>0 do
    local nextByte=value:byte(finalByte+1)
    if not nextByte or nextByte<128 or nextByte>=192 then break end
    finalByte=finalByte-1
  end
  return value:sub(1,finalByte)..'...'
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

-- Online owns no native Beatblock entities. Preserve its shared music manager,
-- but release any Player or transition entity before another state takes over.
local function clearNativeEntities(self)
  if em and em.clear then em.clear({self.menuMusicManager}) end
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
  -- Disabled controls are presentation only: keep them out of focus, hit
  -- testing, and callback dispatch rather than relying on server rejection.
  local focused=enabled~=false and register(self,id,x,y,w,h,run) or false
  ui:button(id,x,y,w,h,label,focused,color,enabled)
end

local function chip(self,id,x,y,w,label,selected,run,color)
  local focused=register(self,id,x,y,w,22,run)
  ui:chip(id,x,y,w,label,selected,color,focused)
end

local function setWorkspace(self,name)
  self.workspace=name
  self.focusId='nav_'..name
  self.modal=nil
  if name~='broadcast' then self.broadcastAdvanced=false end
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
      port=tostring(settings.hostPort or 32145), password='', hostParticipating=true,
      validityChecksEnabled=true, requireSameGameBuild=true, autoRequestChartTransfers=false,
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

local function modifierPolicy(room)
  local source=BBT.roomModifierPolicy and BBT.roomModifierPolicy(room)
    or (room and room.modifiers) or DEFAULT_MODIFIERS
  return {
    rate=tonumber(source.rate) or 1.0,
    vfx=source.vfx or 'full',taps=source.taps or 'default',
    sides=source.sides or 'default',barelies=source.barelies or 'default',
    restartOn=source.restartOn or 'none',
  }
end

local function modifiersEditable(room)
  return isHost() and room and (room.lifecycle=='forming' or room.lifecycle=='chart_locked' or room.lifecycle=='ready')
end

local function openModifiers(self)
  local room=currentRoom()
  if not room then return end
  self.modal={
    kind='modifiers',title='HOST-ENFORCED MODIFIERS',values=modifierPolicy(room),
    editable=modifiersEditable(room),returnFocus=self.focusId,
  }
  self.focusId=self.modal.editable and 'modifier_rate_down' or 'modal_cancel'
end

local function submitModifiers(self)
  local modal=self.modal
  if not modal or modal.kind~='modifiers' or not modal.editable then return end
  local requestId=BBT.command('room.modifiers_set',{modifiers=modal.values})
  if requestId then closeModal(self)
  else modal.error=bounded(BBT.lastError or 'MODIFIER POLICY COULD NOT BE SAVED',48) end
end

local function launchSingleChartSelector(official,selectionMode)
  local mode=selectionMode or 'single'
  if official then BBT.openOfficialSelect(mode)
  else BBT.openChartSelect(mode) end
end

local function chooseSingleChartSource(self,official)
  -- Hosting and local verification share this source picker, but only hosting
  -- may replace an ordered set and therefore needs the destructive warning.
  local selectionMode=self.modal and self.modal.selectionMode or 'single'
  local room=currentRoom()
  local entries=room and room.setlist or {}
  if selectionMode=='single' and #entries>0 then
    openConfirm(
      self,
      'REPLACE ORDERED SET',
      'Selecting one chart removes the current ordered set. Continue to Beatblock chart selection?',
      'REPLACE SET',
      function() launchSingleChartSelector(official,selectionMode) end
    )
    return
  end
  closeModal(self)
  launchSingleChartSelector(official,selectionMode)
end

local function openSingleChartSource(self,selectionMode)
  self.modal={
    kind='chart_source',
    title='SELECT SINGLE CHART',
    selectionMode=selectionMode or 'single',
    returnFocus=self.focusId,
  }
  self.focusId='single_chart_official'
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
  local requestId
  if modal.mode=='host' then
    requestId=BBT.command('room.host_request',{
      displayName=values.displayName,name=values.name,password=values.password,
      port=port,hostApproval=true,allowChartTransfers=true,
      autoRequestChartTransfers=values.autoRequestChartTransfers==true,
      hostParticipating=values.hostParticipating~=false,
      validityChecksEnabled=values.validityChecksEnabled~=false,
      requireSameGameBuild=values.requireSameGameBuild~=false,
    })
  else
    requestId=BBT.command('room.join_request',{
      displayName=values.displayName,address=values.address..':'..values.port,
      password=values.password,spectator=modal.spectator,
    })
  end
  if requestId then closeModal(self)
  else modal.error=bounded(BBT.lastError or 'ONLINE ACTION COULD NOT START',48) end
end

local function header(self)
  ui:text('BEATBLOCK ONLINE',12,7,220,'left','white')
  local runtimeStatus=BBT.companionConnected and 'READY' or (BBT.runtimeStarting and 'STARTING' or 'OFFLINE')
  local status='v'..tostring(BBT.version or 'UNKNOWN')..'  /  '..runtimeStatus
  local statusTone=BBT.companionConnected and 'green' or (BBT.runtimeStarting and 'yellow' or 'red')
  ui:text(status,280,7,308,'right',statusTone)
  local room=currentRoom()
  local primary=Dashboard.primary(context())
  ui:panel(12,27,576,44)
  -- Two 18px lines need a 19px pitch inside the 44px session strip. The
  -- session action is wide enough for the longest primary label at 16pt.
  if room then
    local chart=room.chart
    ui:text(room.name or 'ONLINE SESSION',22,31,220,'left','muted')
    ui:text(chart and (chart.songName or chart.packageName) or 'NO CHART SELECTED',22,50,272,'left',chart and 'white' or 'yellow')
    if chart and primary.id~='request_chart' then
      -- Ends 5px before the session action. The button paints an opaque fill, so
      -- a right-aligned run that reached x=419 lost its last glyph under it.
      ui:text((chart.variant or '')..(chart.official and '  /  OFFICIAL' or '  /  CUSTOM'),280,31,129,'right','muted')
    end
  else
    ui:text('ONLINE SESSION',22,31,220,'left','muted')
    ui:text(BBT.companionConnected and 'READY TO CONNECT' or 'LOCAL RUNTIME REQUIRED',22,50,272,'left',BBT.companionConnected and 'green' or 'yellow')
  end
  if primary.id=='request_chart' then
    button(self,'session_local_chart',295,36,114,26,'FIND LOCAL',function()
      local active=currentRoom()
      if active and active.chart then
        openSingleChartSource(self,'verify')
      end
    end,'cyan')
  end
  button(self,'session_primary',414,36,164,26,primary.label,function() runPrimary(self,primary) end,primary.tone,primary.enabled)
end

function runPrimary(self,item)
  if item.id=='host_room' then openForm(self,'host')
  elseif item.id=='open_installer' then BBT.openInstaller()
  elseif item.id=='select_chart' or item.id=='select_next_chart' then openSingleChartSource(self)
  elseif item.id=='locate_chart' then
    local room=currentRoom()
    if room and room.chart then
      if room.chart.official then BBT.openOfficialSelect('verify') else BBT.openChartSelect('verify') end
    end
  elseif item.id=='request_chart' then
    local room=currentRoom()
    if room and room.chart then
      BBT.command('chart.transfer_request',{chartHash=room.chart.hash})
    end
  elseif item.id=='ready' then BBT.command('room.ready_request',{ready=true})
  elseif item.id=='start_race' then BBT.command('room.start_request',{force=false})
  elseif item.id=='advance_set' then
    local room=currentRoom()
    self.advancePreviousHash=room and room.chart and room.chart.hash
    self.advanceRequestId=BBT.command('setlist.advance',{})
    if self.advanceRequestId then setWorkspace(self,'setlist') end
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
      y=y+ACTION_PITCH
      button(self,'participant_reject',x,y,w,24,'REJECT',function()
        openConfirm(self,'REJECT REQUEST','Reject '..bounded(target.displayName,32)..' from this room?','REJECT',function()
          BBT.command('room.admission_set',{sessionId=target.sessionId,admit=false,role=target.role})
        end)
      end,'red')
      return y+ACTION_PITCH
    end
    button(self,'participant_role',x,y,w,24,target.role=='spectator' and 'MAKE PLAYER' or 'MAKE SPECTATOR',function()
      BBT.command('room.role_set',{sessionId=target.sessionId,role=target.role=='spectator' and 'player' or 'spectator'})
    end,'yellow',room.lifecycle~='playing' and room.lifecycle~='countdown')
    y=y+ACTION_PITCH
    if target.role=='spectator' then
      button(self,'participant_commentator',x,y,w,24,target.commentatorAccess and 'REVOKE COMMENTATOR' or 'GRANT COMMENTATOR',function()
        BBT.command('room.commentator_set',{sessionId=target.sessionId,enabled=not target.commentatorAccess})
      end,target.commentatorAccess and 'yellow' or 'cyan')
      y=y+ACTION_PITCH
    end
    button(self,'participant_remove',x,y,w,24,'REMOVE',function()
      openConfirm(self,'REMOVE PARTICIPANT','Remove '..bounded(target.displayName,32)..' from the room?','REMOVE',function()
        BBT.command('room.kick',{sessionId=target.sessionId})
      end)
    end,'red',room.lifecycle~='playing' and room.lifecycle~='countdown')
  elseif target.sessionId==(BBT.context and BBT.context.sessionId) then
    if isHost() then
      local participating=target.role~='spectator'
      local editable=room.lifecycle=='forming' or room.lifecycle=='chart_locked' or room.lifecycle=='ready'
      button(self,'participant_host_play',x,y,w,24,participating and 'DIRECT NEXT RACE' or 'PLAY NEXT RACE',function()
        BBT.command('room.host_play_set',{participating=not participating})
      end,participating and 'yellow' or 'green',editable)
      y=y+ACTION_PITCH
    end
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
      y=y+ACTION_PITCH
      button(self,'participant_transfer_trust',x,y,w,24,'TRUST THIS ROOM',function()
        BBT.command('chart.transfer_decision',{
          requestId=transfer.requestId,accept=true,trustRoom=true,
          executableContentConfirmed=false,
        })
      end,'cyan',not transfer.containsExecutableContent)
      y=y+ACTION_PITCH
    elseif target.role~='spectator' and not target.verified and room.chart then
      -- Validation accepts candidates from either native chart source. The
      -- runtime still compares the authoritative fingerprint, so exposing
      -- Freeplay here cannot verify the wrong chart by accident.
      local sourceGap=4
      local sourceWidth=math.floor((w-sourceGap)/2)
      button(self,'participant_freeplay',x,y,sourceWidth,24,'FREEPLAY',function()
        BBT.openOfficialSelect('verify')
      end,room.chart.official and 'cyan' or 'white')
      button(self,'participant_locate',x+sourceWidth+sourceGap,y,w-sourceWidth-sourceGap,24,'CUSTOM',function()
        BBT.openChartSelect('verify')
      end,room.chart.official and 'white' or 'cyan')
      y=y+ACTION_PITCH
      if not isHost() then
        button(self,'participant_transfer',x,y,w,24,'REQUEST HOST TRANSFER',function()
          BBT.command('chart.transfer_request',{chartHash=room.chart.hash})
        end,'yellow',room.allowChartTransfers~=false
          and not room.chart.official and room.chart.transferMode=='host_transfer')
        y=y+ACTION_PITCH
      end
    elseif target.role~='spectator' and target.ready
      and (room.lifecycle=='forming' or room.lifecycle=='chart_locked' or room.lifecycle=='ready') then
      button(self,'participant_unready',x,y,w,24,'UNREADY',function() BBT.command('room.ready_request',{ready=false}) end,'yellow')
      y=y+ACTION_PITCH
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
    local width=filter.width
    chip(self,'filter_'..filter.id,x,104,width,filter.label,self.rosterFilter==filter.id,function()
      self.rosterFilter=filter.id
      self.rosterOffset=0
      local selectedPlayer=Dashboard.selectedParticipant(context(),filter.id,self.selectedSessionId)
      self.selectedSessionId=selectedPlayer and selectedPlayer.sessionId or nil
    end,filter.id=='pending' and 'yellow' or 'cyan')
    x=x+width+5
  end
  local target,list=selected(self)
  local selectedIndex=1
  for index,participant in ipairs(list) do
    if participant.sessionId==self.selectedSessionId then selectedIndex=index; break end
  end
  local maxOffset=math.max(0,(math.ceil(#list/ROSTER_PAGE_SIZE)-1)*ROSTER_PAGE_SIZE)
  self.rosterOffset=math.max(0,math.min(maxOffset,self.rosterOffset or 0))
  if selectedIndex<=self.rosterOffset or selectedIndex>self.rosterOffset+ROSTER_PAGE_SIZE then
    self.rosterOffset=math.floor((selectedIndex-1)/ROSTER_PAGE_SIZE)*ROSTER_PAGE_SIZE
  end
  local lifecycle=room.lifecycle
  local showScore=lifecycle=='playing' or lifecycle=='results' or lifecycle=='set_complete'
  ui:text('NAME',20,129,142,'left','muted')
  if showScore then
    ui:text('RANK',205,129,48,'right','muted'); ui:text('ACCURACY',267,129,92,'right','muted')
  else ui:text('STATE',238,129,121,'right','muted') end
  for row=1,ROSTER_PAGE_SIZE do
    local index=self.rosterOffset+row
    local participant=list[index]
    if not participant then break end
    local rowY=147+(row-1)*21
    local focused=self.selectedSessionId==participant.sessionId
    register(self,'participant_'..participant.sessionId,18,rowY,346,20,function()
      self.selectedSessionId=participant.sessionId
    end)
    if focused then ui:color('raised'); love.graphics.rectangle('fill',18,rowY,346,20,2,2) end
    local participantIsHost=room and participant.sessionId==room.hostSessionId
    local role=participantIsHost and '[H]' or (participant.role=='spectator' and (participant.commentatorAccess and '[C]' or '[S]') or '[P]')
    ui:text(role..' '..participant.displayName,23,rowY+1,174,'left',focused and 'black' or 'white')
    if showScore then
      local score=Dashboard.score(participant,lifecycle)
      ui:text(score.rank or '—',185,rowY+1,72,'right',focused and 'black' or score.tone or 'white')
      ui:text(score.accuracy or '—',265,rowY+1,94,'right',focused and 'black' or score.tone or 'white')
    else
      local label,color=Dashboard.participantStatus(participant)
      ui:text(label,226,rowY+1,133,'right',focused and 'black' or color)
    end
  end
  if #list==0 then ui:text('NO PARTICIPANTS IN THIS FILTER',30,178,324,'center','muted') end
  if #list>ROSTER_PAGE_SIZE then
    local page=math.floor(self.rosterOffset/ROSTER_PAGE_SIZE)+1
    local pages=math.ceil(#list/ROSTER_PAGE_SIZE)
    button(self,'roster_previous',20,276,34,22,'<',function()
      self.rosterOffset=math.max(0,self.rosterOffset-ROSTER_PAGE_SIZE)
      local participant=list[self.rosterOffset+1]
      if participant then self.selectedSessionId=participant.sessionId end
    end,'white',self.rosterOffset>0)
    ui:text('PAGE '..page..' / '..pages,60,278,259,'center','muted')
    button(self,'roster_next',325,276,34,22,'>',function()
      self.rosterOffset=math.min(maxOffset,self.rosterOffset+ROSTER_PAGE_SIZE)
      local participant=list[self.rosterOffset+1]
      if participant then self.selectedSessionId=participant.sessionId end
    end,'white',self.rosterOffset<maxOffset)
  end
  return target
end

local function drawInspector(self,target)
  local room=currentRoom()
  ui:panel(379,78,209,225,'PARTICIPANT')
  if not target then
    ui:wrapped('Select a participant to inspect their role, connection, chart verification, and host actions.',391,108,185,6,'muted')
    return
  end
  ui:text(target.displayName,391,104,185,'left','cyan')
  local targetIsHost=room and target.sessionId==room.hostSessionId
  -- The value column is 103px at 16pt, so the host role reads as a verb pair
  -- instead of being ellipsized mid-word.
  local role=targetIsHost and (target.role=='spectator' and 'HOST DIRECTS' or 'HOST PLAYS')
    or (target.role=='spectator' and (target.commentatorAccess and 'COMMENTATOR' or 'SPECTATOR') or 'PLAYER')
  local labels={
    {'ROLE',role}, {'CONNECTION',target.connected==false and 'OFFLINE' or 'CONNECTED'},
    {'CHART',target.role=='spectator' and 'NOT REQUIRED' or (target.verified and 'VERIFIED' or 'MISMATCH')},
    {'RUN',target.validity=='dnf' and 'DNF' or target.validity=='invalid' and 'INVALID'
      or target.role=='spectator' and (targetIsHost and 'DIRECTING' or 'WATCHING')
      or target.ready and 'READY' or 'WAITING'},
  }
  if room and (room.lifecycle=='results' or room.lifecycle=='set_complete') and target.role~='spectator' then
    labels[#labels+1]={'SET TOTAL',target.setTotal and string.format('%.2f',target.setTotal) or '—'}
  end
  -- Flow the detail rows and the action stack from one cursor. An 18px line in
  -- a 17px row printed over the next label, and fixed action offsets pushed the
  -- final button through the panel floor.
  local rowY=126
  for _,item in ipairs(labels) do
    ui:text(item[1],391,rowY,82,'left','muted')
    ui:text(item[2],473,rowY,103,'right',(item[2]=='MISMATCH' or item[2]=='INVALID' or item[2]=='DNF') and 'red' or 'white')
    rowY=rowY+19
  end
  local transfer=BBT.chartTransfer
  local actionY=rowY+4
  if transfer and target.sessionId==(BBT.context and BBT.context.sessionId) then
    local copy=transfer.state=='progress' and ('TRANSFER '..tostring(transfer.percent or 0)..'%')
      or transfer.state=='offer' and 'TRANSFER OFFER AVAILABLE'
      or transfer.state=='consent' and 'CONSENT REQUIRED'
      or nil
    if copy then
      ui:text(copy,391,actionY,185,'left',transfer.state=='progress' and 'cyan' or 'yellow')
      actionY=actionY+20
    end
  end
  if target.validity=='invalid' or target.validity=='dnf' then
    button(self,'participant_run_details',391,actionY,185,24,'RUN DETAILS',function()
      self.modal={kind='details',title=target.validity=='invalid' and 'INVALID RUN' or 'DID NOT FINISH',message=target.invalidReason or 'The runtime did not provide a detailed reason for this result.',returnFocus=self.focusId}
    end,'white')
    actionY=actionY+ACTION_PITCH
  end
  participantActionButtons(self,target,391,actionY,185)
end

local function drawConnect(self)
  ui:panel(12,78,576,225,'CONNECT')
  ui:text('CHOOSE HOW YOU JOIN',24,103,552,'center','cyan')
  ui:wrapped('Create a direct-IP room with the session action above, or join below.',50,124,500,1,'muted')

  ui:text('PLAYER',32,147,250,'left','white')
  ui:wrapped('Compete, verify the locked chart, then ready up.',32,166,250,2,'muted')
  ui:text('SPECTATOR',318,147,250,'left','white')
  ui:wrapped('Watch rankings without scoring. Commentator is host-granted.',318,166,250,2,'muted')

  button(self,'connect_join',32,207,250,30,'JOIN AS PLAYER',function() openForm(self,'join',false) end,'cyan',BBT.companionConnected)
  button(self,'connect_spectate',318,207,250,30,'JOIN AS SPECTATOR',function() openForm(self,'join',true) end,'white',BBT.companionConnected)
  button(self,'connect_exit',418,262,150,28,'EXIT ONLINE',function()
    openConfirm(self,'EXIT ONLINE','Stop the Online runtime and return to the main menu?','EXIT',function() BBT.exitOnline(); leaveToMenu(self) end)
  end,'red')
  local problem=BBT.lastError
  if not problem and not BBT.companionConnected then
    problem=BBT.runtimeLaunchStatus or 'The local runtime is unavailable.'
  end
  if problem then
    -- Three 380px lines is the real budget beside the exit action at 16pt.
    ui:wrapped(bounded(problem,165),24,241,380,3,'red')
  else
    ui:text('HOSTING? USE THE SESSION ACTION ABOVE.',24,246,380,'left','muted')
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
  local title='ORDERED SET'
  if room and not isHost() then title='ORDERED SET  /  HOST CONTROLLED'
  elseif room and (room.lifecycle=='playing' or room.lifecycle=='countdown') then
    title='ORDERED SET  /  LOCKED DURING RACE'
  end
  ui:panel(12,78,576,225,title)
  if not room then
    ui:wrapped('Join or host a room before building an ordered set.',24,111,552,3,'muted')
    return
  end
  local entries=room.setlist or {}
  local selectedEntry,selection=Dashboard.selectedSetlistEntry(
    entries,self.selectedSetlistEntryId,self.setlistSelection
  )
  self.setlistSelection=selection or 1
  self.selectedSetlistEntryId=selectedEntry and selectedEntry.id or nil
  self.setlistSelection,self.setlistOffset=Dashboard.scroll(
    self.setlistSelection,self.setlistOffset,#entries,0,6
  )
  ui:text('ORDER',20,103,46,'center','muted')
  ui:text('CHART',68,103,284,'left','muted')
  ui:text('VARIANT',360,103,102,'right','muted')
  ui:text('STATE',470,103,100,'right','muted')
  for visibleIndex=1,6 do
    local index=self.setlistOffset+visibleIndex
    local entry=entries[index]
    if not entry then break end
    local y=119+(visibleIndex-1)*22
    local active=room.currentSetlistIndex==index-1
    local selected=self.setlistSelection==index
    local state,stateTone=Dashboard.setlistEntryState(entries,index,room.currentSetlistIndex,room.lifecycle)
    register(self,'setlist_entry_'..tostring(entry.id),20,y,560,22,function()
      self.setlistSelection=index
      self.selectedSetlistEntryId=entry.id
    end)
    if active then ui:color('raised'); love.graphics.rectangle('fill',20,y,560,22,2,2) end
    if selected then ui:color('cyan'); love.graphics.rectangle('line',20.5,y+.5,559,21,2,2) end
    local rowTone=active and 'black' or 'white'
    ui:text(tostring(index),20,y+2,46,'center',rowTone)
    ui:text(entry.chart.songName or entry.chart.packageName or 'Chart',68,y+2,284,'left',rowTone)
    ui:text(entry.chart.variant or '',360,y+2,102,'right',active and 'black' or 'muted')
    ui:text(state,470,y+2,100,'right',active and 'black' or stateTone)
  end
  if #entries==0 then ui:text('NO CHARTS IN THE ORDERED SET',28,160,544,'center','muted') end
  local canEdit=isHost() and room.lifecycle~='playing' and room.lifecycle~='countdown'
  button(self,'setlist_add_official',20,254,276,22,'ADD OFFICIAL',function()
    BBT.openOfficialSelect('setlist')
  end,'green',canEdit)
  button(self,'setlist_add_custom',304,254,276,22,'ADD CUSTOM',function()
    BBT.openChartSelect('setlist')
  end,'green',canEdit)

  selection=self.setlistSelection or 1
  selectedEntry=entries[selection]
  local activeSelection=room.currentSetlistIndex and room.currentSetlistIndex+1 or nil
  local function canMoveTo(target)
    local targetEntry=entries[target]
    if not selectedEntry or not targetEntry or selectedEntry.completed or targetEntry.completed then return false end
    return not activeSelection or (selection>activeSelection and target>activeSelection)
  end
  button(self,'setlist_up',20,279,181,22,'MOVE UP',function()
    local target=selection-1
    BBT.command('setlist.move',{from=selection-1,to=target-1})
  end,'white',canEdit and canMoveTo(selection-1))
  button(self,'setlist_down',209,279,181,22,'MOVE DOWN',function()
    local target=selection+1
    BBT.command('setlist.move',{from=selection-1,to=target-1})
  end,'white',canEdit and canMoveTo(selection+1))
  button(self,'setlist_remove',398,279,182,22,'REMOVE',function()
    local entry=entries[selection]
    openConfirm(self,'REMOVE CHART','Remove '..bounded(entry and (entry.chart.songName or entry.chart.packageName) or 'this chart',28)..' from the setlist?','REMOVE',function()
      BBT.command('setlist.remove',{index=selection-1})
    end)
  end,'red',canEdit and #entries>0 and not (
    room.currentSetlistIndex==selection-1
    and (room.lifecycle=='results' or room.lifecycle=='set_complete')
  ))
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

local function loadBroadcastDraft(self,id)
  local slot=rendererSlot(id or self.broadcastSlot or 'A')
  self.broadcastSlot=slot.id
  self.broadcastDraft={
    mode=slot.mode or 'full',
    width=slot.width or 1280,
    height=slot.height or 720,
    fps=slot.fps or 60,
    delayMs=slot.delayMs or 500,
  }
end

local function drawBroadcastAdvanced(self,room)
  if not self.broadcastDraft then loadBroadcastDraft(self,'A') end
  local draft=self.broadcastDraft
  local editable=room and room.lifecycle~='playing' and room.lifecycle~='countdown'
  ui:text('ADVANCED OBS EXPORT',24,105,220,'left','cyan')
  ui:text('STREAM '..self.broadcastSlot,356,105,220,'right','white')

  ui:text('STREAM',24,134,66,'left','muted')
  for index,id in ipairs(STREAMS) do
    chip(self,'broadcast_slot_'..id,96+(index-1)*60,132,54,id,self.broadcastSlot==id,function()
      loadBroadcastDraft(self,id)
    end,'cyan')
  end
  ui:text('MODE',24,166,66,'left','muted')
  chip(self,'broadcast_mode_full',96,164,104,'FULL',draft.mode=='full',function() draft.mode='full' end,'cyan')
  chip(self,'broadcast_mode_clean',206,164,104,'CLEAN',draft.mode=='clean',function() draft.mode='clean' end,'cyan')

  ui:text('SIZE',24,198,66,'left','muted')
  chip(self,'broadcast_size_720',96,196,128,'1280 x 720',draft.width==1280,function()
    draft.width=1280; draft.height=720
  end,'cyan')
  chip(self,'broadcast_size_1080',230,196,128,'1920 x 1080',draft.width==1920,function()
    draft.width=1920; draft.height=1080
  end,'cyan')
  ui:text('FPS',374,198,38,'left','muted')
  chip(self,'broadcast_fps_30',416,196,72,'30',draft.fps==30,function() draft.fps=30 end,'cyan')
  chip(self,'broadcast_fps_60',494,196,72,'60',draft.fps==60,function() draft.fps=60 end,'cyan')

  ui:text('DELAY',24,230,66,'left','muted')
  for index,value in ipairs({250,500,1000,1500}) do
    chip(self,'broadcast_delay_'..tostring(value),96+(index-1)*78,228,72,tostring(value)..' MS',draft.delayMs==value,function()
      draft.delayMs=value
    end,'cyan')
  end
  local highLoad=draft.width==1920 and draft.fps==60
  ui:text(highLoad and 'HIGH GPU LOAD' or 'EXPORT CLOCK LOCKED',416,230,150,'right',highLoad and 'yellow' or 'muted')

  button(self,'broadcast_apply',24,267,172,25,'APPLY TO STREAM '..self.broadcastSlot,function()
    local slot=rendererSlot(self.broadcastSlot)
    BBT.command('renderer.configure',{
      slot=self.broadcastSlot,
      participantId=slot.participantId,
      participantName=slot.participantName,
      mode=draft.mode,width=draft.width,height=draft.height,
      fps=draft.fps,delayMs=draft.delayMs,featured=slot.featured,
    })
  end,'green',editable)
  ui:text(draft.mode:upper()..' / '..draft.width..'x'..draft.height..' / '..draft.fps..' FPS',205,274,225,'left','white')
  button(self,'broadcast_advanced_back',446,267,120,25,'BACK',function()
    self.broadcastAdvanced=false
    self.focusId='broadcast_advanced'
  end,'white')
end

local function drawBroadcast(self)
  local allowed,authority=Dashboard.canBroadcast(context())
  ui:panel(12,78,576,225,'BROADCAST')
  if not allowed then
    ui:text('BROADCAST IS NOT AVAILABLE',48,121,504,'center','yellow')
    ui:wrapped('Ordinary Spectators can follow the room and rankings. A host may grant Commentator access from the participant inspector.',95,151,410,5,'muted')
    return
  end
  -- Broadcast lifecycle controls use the same normalized room snapshot as
  -- every other workspace. Keeping it local avoids an accidental lookup of a
  -- nonexistent global when hosts or commentators open the OBS menu.
  local room=currentRoom()
  if authority=='host' and self.broadcastAdvanced then
    drawBroadcastAdvanced(self,room)
    return
  end
  local target=selected(self)
  local rendererEditable=room and room.lifecycle~='playing' and room.lifecycle~='countdown'
  ui:text(authority=='host' and 'HOST PLAN' or 'HOST PLAN  /  READ ONLY',24,105,270,'left','cyan')
  ui:text(target and ('CANDIDATE: '..target.displayName) or 'CANDIDATE: SELECT A PLAYER',306,105,270,'right','muted')
  -- STOP/ASSIGN and FEATURE need 56 and 63 pixels of glyphs at 16pt, so the
  -- four stream cards span the whole panel instead of clipping their own
  -- action labels inside a 132px card.
  for index,id in ipairs(STREAMS) do
    local slot=authority=='host' and rendererSlot(id) or planSlot(id)
    local x=18+(index-1)*142
    ui:color(slot.active and (slot.featured and 'cyan' or 'raised') or 'panel')
    love.graphics.rectangle('fill',x,126,138,104,3,3)
    ui:color('raised'); love.graphics.rectangle('line',x+.5,126.5,137,103,3,3)
    ui:text('STREAM '..id,x+5,132,128,'left',slot.active and 'black' or 'white')
    ui:text(slot.participantName or slot.participant_name or 'UNASSIGNED',x+5,151,128,'left',slot.active and 'black' or 'muted')
    local health=slot.lastError and 'ERROR' or slot.healthy and 'HEALTHY' or slot.active and 'STARTING' or 'STOPPED'
    -- An active card fills light (cyan when featured, raised otherwise) and
    -- `muted` is the same palette value as `raised`, so HEALTHY and STARTING
    -- were painted in the card's own colour and vanished. The two lines above
    -- already switch to black when active; this one was missed. Errors keep red,
    -- which still reads against both light fills.
    ui:text(health,x+5,170,128,'left',
      slot.lastError and 'red' or slot.active and 'black' or slot.healthy and 'green' or 'muted')
    if authority=='host' then
      button(self,'broadcast_assign_'..id,x+5,196,58,26,slot.active and 'STOP' or 'ASSIGN',function()
        if slot.active then BBT.command('renderer.stop',{slot=id})
        elseif target and target.role~='spectator' then
          BBT.command('renderer.configure',{
            slot=id,participantId=target.sessionId,participantName=target.displayName,
            mode=slot.mode,width=slot.width,height=slot.height,fps=slot.fps,
            delayMs=slot.delayMs,featured=slot.featured,
          })
        end
      end,slot.active and 'yellow' or 'cyan',slot.active or (rendererEditable and target and target.role~='spectator'))
      button(self,'broadcast_feature_'..id,x+68,196,65,26,'FEATURE',function()
        BBT.command('renderer.configure',{slot=id,participantId=slot.participantId,participantName=slot.participantName,mode=slot.mode,width=slot.width,height=slot.height,fps=slot.fps,delayMs=slot.delayMs,featured=true})
      end,'green',rendererEditable and slot.active and not slot.featured)
    end
  end
  local featuredActive=false
  for _,slot in ipairs(BBT.renderers or {}) do
    if slot.active and slot.featured then featuredActive=true; break end
  end
  if authority=='commentator' then
    local enabled=BBT.mirrorEnabled or (BBT.runtimeSnapshot and BBT.runtimeSnapshot.mirrorEnabled)
    ui:text('THIS PC',24,242,80,'left','white')
    ui:text(enabled and 'LOCAL MIRROR ENABLED' or 'LOCAL MIRROR DISABLED',112,242,250,'left',enabled and 'green' or 'yellow')
    button(self,'broadcast_mirror',398,238,178,27,enabled and 'DISABLE MIRROR' or 'ENABLE MIRROR',function()
      if enabled then BBT.command('broadcast.mirror_set',{enabled=false})
      else
        openConfirm(self,'ENABLE LOCAL MIRROR','This may start four video renderers plus the host plan autoplay audio renderer and increase CPU/GPU use. Continue?','ENABLE',function()
          BBT.command('broadcast.mirror_set',{enabled=true})
        end)
      end
    end,enabled and 'yellow' or 'cyan')
  else
    local autoplay=(BBT.runtimeSnapshot and BBT.runtimeSnapshot.autoplayAudio) or {}
    button(self,'broadcast_autoplay',24,238,178,27,
      autoplay.enabled and 'DISABLE AUTOPLAY MIX' or 'ENABLE AUTOPLAY MIX',function()
      if autoplay.enabled then
        BBT.command('broadcast.autoplay_audio_set',{enabled=false})
      else
        openConfirm(self,'ENABLE AUTOPLAY MIX',
          'Launch one extra audio-only Beatblock renderer with song audio and one perfect native hitsound per positive scoring opportunity?',
          'ENABLE',function()
            BBT.command('broadcast.autoplay_audio_set',{enabled=true})
          end)
      end
    end,autoplay.enabled and 'yellow' or 'cyan',
      rendererEditable and (autoplay.enabled or featuredActive))
    button(self,'broadcast_advanced',398,238,178,27,'ADVANCED EXPORT',function()
      self.broadcastAdvanced=true
      loadBroadcastDraft(self,self.broadcastSlot or 'A')
    end,'white')
  end
  local detail
  for _,slot in ipairs(BBT.renderers or {}) do if slot.lastError then detail=slot.lastError break end end
  local autoplay=(BBT.runtimeSnapshot and BBT.runtimeSnapshot.autoplayAudio) or {}
  detail=detail or autoplay.error
  if detail then
    ui:text('RENDERER: '..bounded(detail,48),24,274,465,'left','red')
    button(self,'broadcast_details',496,270,80,24,'DETAILS',function()
      self.modal={kind='details',title='RENDERER DETAILS',message=detail,returnFocus=self.focusId}
    end,'white')
  else
    local autoplayLabel=autoplay.enabled and (autoplay.healthy and 'AUTOPLAY MIX HEALTHY' or 'AUTOPLAY MIX STARTING')
      or (authority=='host' and not rendererEditable and 'AUTOPLAY MIX LOCKED DURING RACE'
      or (not featuredActive and 'FEATURE A STREAM TO ENABLE AUTOPLAY MIX' or 'AUTOPLAY MIX OFF'))
    ui:text(autoplayLabel,24,276,552,'left',autoplay.enabled and 'green' or 'muted')
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
  ui:wrapped('History is the archive. Current Results remain in the Room workspace.',413,185,163,4,'muted')
end

local function drawSettings(self)
  ui:panel(12,78,360,225,'SETTINGS')
  ui:panel(379,78,209,225,'COMPATIBILITY')
  local settings=BBT.settings or {}
  local room=currentRoom()
  local checksEnabled=not room or room.validityChecksEnabled~=false
  local sameBuildRequired=not room or room.requireSameGameBuild~=false
  local autoRequests=room and room.autoRequestChartTransfers==true
  local checksEditable=modifiersEditable(room)
  local transferEditable=checksEditable and room.allowChartTransfers~=false
  ui:text('DESKTOP AUDIO',24,104,145,'left','muted')
  ui:text(
    settings.rendererDesktopMute==false and 'DRIVER FALLBACK' or 'EXACT-PID MUTE',
    169,104,189,'right',settings.rendererDesktopMute==false and 'yellow' or 'green'
  )
  ui:text('TRANSFER CACHE',24,124,145,'left','muted')
  ui:text(
    tostring((BBT.runtimeSnapshot and BBT.runtimeSnapshot.chartCacheSizeLabel) or '0 MB / 2 GB'),
    169,124,189,'right','white'
  )
  button(self,'settings_hud',24,146,164,25,
    settings.hudEnabled==false and 'HUD: OFF' or 'HUD: ON',function()
    BBT.command('settings.update',{hudEnabled=not (settings.hudEnabled~=false)})
  end,'cyan')
  button(self,'settings_validity',196,146,164,25,
    checksEnabled and 'RUN CHECKS: ON' or 'RUN CHECKS: OFF',function()
    local run=function() BBT.command('room.validity_checks_set',{enabled=not checksEnabled}) end
    if checksEnabled then
      openConfirm(self,'DISABLE RUN CHECKS','Retries and missing score events will not invalidate plays. Counter bounds and DNF completion rules remain active.','DISABLE',run)
    else run() end
  end,checksEnabled and 'yellow' or 'green',checksEditable)
  button(self,'settings_build_policy',24,177,164,25,
    sameBuildRequired and 'BUILD: SAME' or 'BUILD: ANY',function()
    local run=function()
      BBT.command('room.game_build_policy_set',{required=not sameBuildRequired})
    end
    if sameBuildRequired then
      openConfirm(self,'ALLOW MIXED BUILDS','Players on different Beatblock builds may have different chart data or judgement windows. Continue for this room?','ALLOW',run)
    else run() end
  end,sameBuildRequired and 'yellow' or 'green',checksEditable)
  button(self,'settings_transfer_policy',196,177,164,25,
    autoRequests and 'REQUESTS: AUTO' or 'REQUESTS: MANUAL',function()
    BBT.command('room.chart_transfer_policy_set',{autoRequest=not autoRequests})
  end,autoRequests and 'green' or 'cyan',transferEditable)
  local configured=modifierPolicy(room)
  local modifiersDefault=configured.rate==1 and configured.vfx=='full' and configured.taps=='default'
    and configured.sides=='default' and configured.barelies=='default' and configured.restartOn=='none'
  button(self,'settings_modifiers',24,216,164,25,
    modifiersDefault and 'MODIFIERS: DEFAULT' or 'MODIFIERS: CUSTOM',function()
    openModifiers(self)
  end,modifiersDefault and 'cyan' or 'yellow',room~=nil)
  button(self,'settings_renderer_mute',196,216,164,25,
    settings.rendererDesktopMute==false and 'DESKTOP MUTE: OFF' or 'DESKTOP MUTE: ON',function()
    BBT.command('settings.update',{rendererDesktopMute=settings.rendererDesktopMute==false})
  end,settings.rendererDesktopMute==false and 'green' or 'yellow')
  button(self,'settings_clear_cache',24,247,336,25,'CLEAR TRANSFER CACHE',function()
    openConfirm(self,'CLEAR TRANSFER CACHE','Remove inactive BBT-managed chart packages? The active chart is protected.','CLEAR',function()
      BBT.command('chart.cache_clear',{})
    end)
  end,'yellow')
  local diagnostics=BBT.diagnostics or {}
  local runtimeVersion=tostring(diagnostics.runtimeVersion or 'OFFLINE')
  local runtimeMatch=BBT.companionConnected and runtimeVersion==tostring(BBT.version)
  local protocolVersion=tonumber(diagnostics.protocolVersion) or tonumber(BBT.protocolVersion) or 0
  local protocolMatch=BBT.companionConnected and protocolVersion==tonumber(BBT.protocolVersion)
  local testedVersion=tostring(diagnostics.testedBeatblockVersion or BBT.testedBeatblockVersion or 'UNKNOWN')
  local detectedVersion=tostring(diagnostics.detectedBeatblockVersion or 'START GAME')
  local detectedBuild=tostring(diagnostics.detectedBeatblockBuildId or '')
  local compatibilityRows={
    {'ONLINE','v'..tostring(BBT.version or 'UNKNOWN'),'white'},
    {'RUNTIME',runtimeVersion=='OFFLINE' and runtimeVersion or 'v'..runtimeVersion,runtimeMatch and 'green' or 'yellow'},
    {'PROTOCOL','V'..tostring(protocolVersion)..(protocolMatch and ' / MATCH' or ' / CHECK'),protocolMatch and 'green' or 'yellow'},
    {'TESTED ON',testedVersion..'+','cyan'},
  }
  for index,row in ipairs(compatibilityRows) do
    local y=106+(index-1)*22
    ui:text(row[1],391,y,72,'left','muted')
    ui:text(row[2],463,y,113,'right',row[3])
  end
  -- 18px lines need a 19px pitch; the detected game and build lines printed
  -- over each other on a 13px one.
  ui:text('GAME '..bounded(detectedVersion,24),391,193,185,'left','muted')
  if detectedBuild~='' then ui:text('BUILD ['..detectedBuild:sub(1,12)..']',391,212,185,'left','cyan') end
  button(self,'settings_logs',391,234,89,25,'LOGS',function() BBT.command('paths.open_logs',{}) end,'white')
  button(self,'settings_exports',487,234,89,25,'EXPORTS',function() BBT.command('paths.open_exports',{}) end,'white')
  button(self,'settings_diagnostics',391,264,185,25,'REFRESH DIAGNOSTICS',function() BBT.command('diagnostics.get',{}) end,'cyan')
end

-- Help is two rows of two columns. Copy is stored, not positioned, so the
-- sections are measured and flowed at the live font size instead of trusting
-- offsets that were tuned for a smaller QA font.
local HELP_SECTIONS = {
  {'ROOM ROLES','Player competes. Spectator watches. Commentator is a host-granted Spectator permission that can mirror the Host Plan to this PC.'},
  {'CHARTS & TRANSFER','Charts resolve locally first. Players may request a package; hosts automate only the request. Consent is local; scripts need confirmation.'},
  -- Separator is '/' to match the footer legend. DigitalDisco-Thin has no U+2022,
  -- so a bullet renders as a missing-glyph box at the game's real font.
  {'CONTROLS','Arrows navigate  /  Enter selects  /  Esc returns one layer  /  Mouse uses the same focus.'},
  {'TROUBLESHOOTING','Open Logs for full runtime errors. Broadcast error summaries stay bounded so controls never disappear.'},
}
local HELP_COLUMNS = {24,310}
local HELP_COLUMN_WIDTH = 266

local function drawHelp(self)
  ui:panel(12,78,576,225,'HELP')
  local font=love.graphics.getFont()
  local lineHeight=font:getHeight()
  local pitch=lineHeight+1
  local floorY=299
  local y=106
  for row=1,2 do
    local first=(row-1)*#HELP_COLUMNS+1
    local bodyY=y+lineHeight+4
    -- Never print past the panel: the remaining rows decide the line budget,
    -- and the audit reports the shortfall instead of hiding it in an ellipsis.
    local budget=math.max(0,math.floor((floorY-bodyY-lineHeight)/pitch)+1)
    local used=0
    for column=1,#HELP_COLUMNS do
      local section=HELP_SECTIONS[first+column-1]
      local x=HELP_COLUMNS[column]
      ui:text(section[1],x,y,HELP_COLUMN_WIDTH,'left','cyan')
      local _,lines=font:getWrap(section[2],HELP_COLUMN_WIDTH)
      local shown=math.min(budget,#lines)
      if shown>used then used=shown end
      ui:wrapped(section[2],x,bodyY,HELP_COLUMN_WIDTH,shown,'muted')
    end
    y=bodyY+used*pitch+5
  end
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
  local problem=BBT.lastError
  local hint=problem and bounded(problem,110)
    or (self.modal and 'ESC: CLOSE  /  ENTER: SELECT' or 'ARROWS: NAVIGATE  /  ENTER: SELECT  /  ESC: BACK')
  ui:text(hint,12,333,576,'center',problem and 'red' or 'muted')
end

local function drawModal(self)
  if not self.modal then return end
  ui:veil()
  local modal=self.modal
  if modal.kind=='form' then
    -- The host form reserves one explicit row for the owner's race role. Keep
    -- it separate from editable fields so text input and deletion continue to
    -- target only the last selected text box.
    local isHostForm=modal.mode=='host'
    ui:panel(118,isHostForm and 7 or 60,364,isHostForm and 346 or 240,modal.title)
    for index,key in ipairs(modal.fields) do
      local y=(isHostForm and 52 or 96)+(index-1)*(isHostForm and 32 or 38)
      local labels={displayName='DISPLAY NAME',name='ROOM NAME',address='HOST ADDRESS',port='UDP PORT',password='PASSWORD'}
      ui:text(labels[key],136,y,124,'left','muted')
      local value=key=='password' and string.rep('*',#modal.values[key]) or modal.values[key]
      button(self,'form_'..key,261,y-5,203,27,value,function() modal.index=index; self.focusId='form_'..key end,'white')
    end
    if isHostForm then
      ui:text('HOST ROLE',136,180,124,'left','muted')
      chip(self,'form_host_play',261,176,96,'PLAY',modal.values.hostParticipating~=false,function()
        modal.values.hostParticipating=true; modal.error=nil
      end,'green')
      chip(self,'form_host_direct',363,176,101,'DIRECT',modal.values.hostParticipating==false,function()
        modal.values.hostParticipating=false; modal.error=nil
      end,'green')
      ui:text('RUN CHECKS',136,210,124,'left','muted')
      chip(self,'form_checks_on',261,206,96,'ON',modal.values.validityChecksEnabled~=false,function()
        modal.values.validityChecksEnabled=true; modal.error=nil
      end,'green')
      chip(self,'form_checks_off',363,206,101,'OFF',modal.values.validityChecksEnabled==false,function()
        modal.values.validityChecksEnabled=false; modal.error=nil
      end,'yellow')
      ui:text('SAME BUILD',136,240,124,'left','muted')
      chip(self,'form_build_same',261,236,96,'REQUIRE',modal.values.requireSameGameBuild~=false,function()
        modal.values.requireSameGameBuild=true; modal.error=nil
      end,'green')
      chip(self,'form_build_any',363,236,101,'ALLOW ANY',modal.values.requireSameGameBuild==false,function()
        modal.values.requireSameGameBuild=false; modal.error=nil
      end,'yellow')
      local note=modal.values.requireSameGameBuild==false and 'CASUAL / MIXED BUILDS'
        or (modal.values.validityChecksEnabled==false and 'CASUAL / RETRIES ALLOWED'
        or (modal.values.hostParticipating==false and 'COMPETITIVE / HOST DIRECTS' or 'COMPETITIVE / HOST PLAYS'))
      ui:text(note,261,265,203,'center','muted')
    end
    local actionY=isHostForm and 289 or 252
    button(self,'form_submit',261,actionY,98,27,modal.mode=='host' and 'CREATE' or 'JOIN',function() submitForm(self) end,'green')
    button(self,'form_cancel',366,actionY,98,27,'CANCEL',function() closeModal(self) end,'white')
    if modal.error then ui:text(modal.error,136,isHostForm and 322 or 282,328,'center','red') end
  elseif modal.kind=='modifiers' then
    ui:panel(62,7,476,346,modal.title)
    ui:wrapped(
      'This room policy replaces local chart options for Game and Results. Saved preferences are restored afterward.',
      82,38,436,2,'muted'
    )
    local function drawChoices(key,label,y)
      ui:text(label,82,y+2,94,'left','muted')
      local choices=MODIFIER_CHOICES[key]
      local gap=4
      local width=math.floor((334-gap*(#choices-1))/#choices)
      local x=184
      for _,choice in ipairs(choices) do
        local id='modifier_'..key..'_'..choice[1]
        if modal.editable then
          chip(self,id,x,y,width,choice[2],modal.values[key]==choice[1],function()
            modal.values[key]=choice[1]; modal.error=nil
          end,'cyan')
        else
          ui:chip(id,x,y,width,choice[2],modal.values[key]==choice[1],'cyan',false)
        end
        x=x+width+gap
      end
    end
    ui:text('GAME SPEED',82,80,94,'left','muted')
    if modal.editable then
      button(self,'modifier_rate_down',184,76,52,25,'-',function()
        modal.values.rate=math.max(0.5,math.floor((modal.values.rate-0.1)*10+0.5)/10)
        modal.error=nil
      end,'cyan')
      ui:text(string.format('%.1fX',modal.values.rate),242,80,218,'center','white')
      button(self,'modifier_rate_up',466,76,52,25,'+',function()
        modal.values.rate=math.min(5,math.floor((modal.values.rate+0.1)*10+0.5)/10)
        modal.error=nil
      end,'cyan')
    else
      ui:text(string.format('%.1fX',modal.values.rate),184,80,334,'center','cyan')
    end
    drawChoices('vfx','VFX',110)
    drawChoices('taps','TAPS',144)
    drawChoices('sides','SIDES',178)
    drawChoices('barelies','BARELIES',212)
    drawChoices('restartOn','RESTART ON',246)
    local note=modal.error or (modal.editable and 'EDITABLE UNTIL COUNTDOWN' or 'HOST POLICY / READ ONLY')
    ui:text(note,82,280,436,'center',modal.error and 'red' or 'muted')
    if modal.editable then
      button(self,'modifiers_apply',294,310,108,27,'APPLY',function() submitModifiers(self) end,'green')
      button(self,'modal_cancel',410,310,108,27,'CANCEL',function() closeModal(self) end,'white')
    else
      button(self,'modal_cancel',410,310,108,27,'CLOSE',function() closeModal(self) end,'white')
    end
  elseif modal.kind=='chart_source' then
    local room=currentRoom()
    local hasOrderedSet=room and #(room.setlist or {})>0
    local verifying=modal.selectionMode=='verify'
    local replacing=hasOrderedSet and not verifying
    ui:panel(126,84,348,200,modal.title)
    ui:wrapped(
      verifying
        and 'Choose where the locked chart is installed. Online will verify it without changing the ordered set.'
        or hasOrderedSet
        and 'Choose a Beatblock source. You will confirm before this single chart replaces the ordered set.'
        or 'Choose which Beatblock library supplies this one-off room chart.',
      146,117,308,3,replacing and 'yellow' or 'muted'
    )
    button(self,'single_chart_official',146,178,143,27,'OFFICIAL CHART',function()
      chooseSingleChartSource(self,true)
    end,replacing and 'yellow' or 'cyan')
    button(self,'single_chart_custom',311,178,143,27,'CUSTOM CHART',function()
      chooseSingleChartSource(self,false)
    end,replacing and 'yellow' or 'cyan')
    button(self,'modal_cancel',229,225,142,27,'CANCEL',function() closeModal(self) end,'white')
  elseif modal.kind=='details' then
    -- Eight 372px lines is what the panel actually holds at 16pt; the byte
    -- budget matches so the reason is never cut without being audited.
    ui:panel(94,60,412,240,modal.title)
    ui:wrapped(bounded(modal.message,400),114,93,372,8,'red')
    button(self,'modal_cancel',331,265,155,27,'CLOSE',function() closeModal(self) end,'white')
  else
    ui:panel(126,99,348,162,modal.title)
    ui:wrapped(modal.message,146,131,308,4,modal.kind=='details' and 'red' or 'white')
    if modal.kind=='confirm' then
      button(self,'modal_confirm',146,213,143,27,modal.label,function() local run=modal.run; closeModal(self); run() end,'red')
      button(self,'modal_cancel',311,213,143,27,'CANCEL',function() closeModal(self) end,'white')
    else
      button(self,'modal_cancel',311,213,143,27,'CLOSE',function() closeModal(self) end,'white')
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
  BBT.layoutAudit=ui:finish()
end

local function focusIndex(self)
  for index,control in ipairs(self.controls or {}) do if control.id==self.focusId then return index end end
  return 1
end

local function continueAdvancedChart(self)
  local requestId=self.advanceRequestId
  if not requestId or BBT.pendingRequestId==requestId then return false end
  self.advanceRequestId=nil
  if BBT.lastCompletedRequestId~=requestId then
    self.advancePreviousHash=nil
    return false
  end
  local room=currentRoom()
  local changed=room and room.chart and room.chart.hash~=self.advancePreviousHash
  self.advancePreviousHash=nil
  local me=BBT.currentPlayer()
  if not changed or not isHost() or not me or me.role=='spectator' then return false end
  -- Future setlist entries are chosen when the set is built, but the active
  -- game selection remains on the completed chart. Re-open the appropriate
  -- selector now so the host verifies and launches the newly active entry.
  if room.chart.official then BBT.openOfficialSelect('verify')
  else BBT.openChartSelect('verify') end
  return true
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
    elseif self.workspace=='broadcast' and self.broadcastAdvanced then
      self.broadcastAdvanced=false
      self.focusId='broadcast_advanced'
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
  function st:openSingleChartSource(selectionMode) openSingleChartSource(self,selectionMode) end
  function st:chooseSingleChartSource(official) chooseSingleChartSource(self,official) end
  function st:openModifiers() openModifiers(self) end
  function st:submitModifiers() submitModifiers(self) end
  st:setInit(function(self,options)
    options=options or {}
    -- Game/Results retain the enforced view long enough for native judgement
    -- and result labeling; returning to Online restores the player's own save.
    if BBT.restoreRoomModifiers then BBT.restoreRoomModifiers() end
    applyBeatblockPalette(false)
    applyBeatblockMenuFont()
    local initialWorkspace=options.workspace
    local room=currentRoom()
    if not initialWorkspace and isHost() and room
      and (room.lifecycle=='results' or room.lifecycle=='set_complete') then
      initialWorkspace='setlist'
    end
    self.workspace=initialWorkspace or 'room'; self.rosterFilter='all'; self.selectedSessionId=nil
    self.focusId=self.workspace=='room' and 'session_primary' or 'nav_'..self.workspace
    self.controls={}; self.broadcastAdvanced=false; self.broadcastSlot='A'; self.broadcastDraft=nil
    self.setlistSelection=1; self.selectedSetlistEntryId=nil; self.setlistOffset=0; self.rosterOffset=0
    self.advanceRequestId=nil; self.advancePreviousHash=nil; self.modal=nil
    -- Online is a complete state, not a menu modal. Suppress the entity
    -- manager retained from Menu and clear those entities before Song Select
    -- can inherit them on the next transition.
    self.holdEntityDraw=true
    clearNativeEntities(self)
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
    -- State changes call leave before initializing the destination. Clear any
    -- retained Player now so it cannot keep updating beside the next state's
    -- newly created Player instance.
    clearNativeEntities(self)
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
    if continueAdvancedChart(self) then return end
    update(self)
  end)
  st:setBgDraw(function(self)
    ui:color('black')
    love.graphics.rectangle('fill',0,0,project.res.x,project.res.y)
  end)
  st:setFgDraw(function(self) draw(self); ui:color('white') end)
  return st
end
