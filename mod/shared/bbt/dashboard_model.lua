-- Pure Online-dashboard decisions. Keeping this module free of LÖVE globals
-- makes the role/state hierarchy executable in tests with Beatblock's Lua 5.1.
local Dashboard = {}

local function activeRoom(context)
  local value = context.room
  if value and value.id ~= 'offline' and value.lifecycle ~= 'closed' then return value end
  return nil
end

function Dashboard.phase(context)
  if not context.runtimeReady then
    return context.runtimeStarting and 'runtime_starting' or 'runtime_error'
  end
  local current = activeRoom(context)
  if not current then return 'connect' end
  if current.lifecycle == 'results' or current.lifecycle == 'set_complete' then return 'results' end
  return 'lobby'
end

function Dashboard.summary(context)
  local summary = {players=0,spectators=0,ready=0,verified=0,pending=0,connected=0,allReady=false}
  local current = activeRoom(context)
  for _, participant in ipairs(current and current.participants or {}) do
    local connected = participant.connected ~= false
    if connected then
      summary.connected = summary.connected + 1
      if not participant.admitted then summary.pending = summary.pending + 1
      elseif participant.role == 'spectator' then summary.spectators = summary.spectators + 1
      else
        summary.players = summary.players + 1
        if participant.ready then summary.ready = summary.ready + 1 end
        if participant.verified then summary.verified = summary.verified + 1 end
      end
    end
  end
  summary.allReady = summary.players > 0 and summary.ready == summary.players and summary.verified == summary.players
  return summary
end

function Dashboard.participantStatus(participant)
  if not participant.admitted then return 'PENDING','yellow' end
  if participant.connected == false then return 'OFFLINE','red' end
  if participant.validity == 'dnf' then return 'DNF','red' end
  if participant.validity == 'invalid' then return 'INVALID','red' end
  if participant.role == 'spectator' then return 'WATCHING','cyan' end
  if participant.ready then return 'READY','green' end
  if participant.verified then return 'VERIFIED','cyan' end
  return 'CHECK CHART','yellow'
end

function Dashboard.visibleParticipants(context, filter)
  local visible = {}
  local current = activeRoom(context)
  for _,participant in ipairs(current and current.participants or {}) do
    local pending = participant.admitted ~= true
    local include = filter == nil or filter == 'all'
      or (filter == 'pending' and pending)
      or (filter == 'players' and not pending and participant.role ~= 'spectator')
      or (filter == 'spectators' and not pending and participant.role == 'spectator')
    if include then visible[#visible+1] = participant end
  end
  table.sort(visible,function(a,b)
    if (a.admitted ~= true) ~= (b.admitted ~= true) then return a.admitted ~= true end
    local aHost=a.sessionId==current.hostSessionId
    local bHost=b.sessionId==current.hostSessionId
    if aHost~=bHost then return aHost end
    -- Host and Player are both racing roles. Comparing their raw role strings
    -- made the ordering non-transitive (Host < Player and Player < Host), which
    -- can crash Lua's table.sort as soon as the real Host role is present.
    local aSpectator=a.role=='spectator'
    local bSpectator=b.role=='spectator'
    if aSpectator~=bSpectator then return not aSpectator end
    return string.lower(a.displayName or '') < string.lower(b.displayName or '')
  end)
  return visible
end

function Dashboard.selectedParticipant(context, filter, sessionId)
  local visible = Dashboard.visibleParticipants(context,filter)
  for _,participant in ipairs(visible) do
    if participant.sessionId == sessionId then return participant,visible end
  end
  return visible[1],visible
end

function Dashboard.score(participant, lifecycle)
  if not participant or participant.role == 'spectator' then return {rank='—',accuracy='—'} end
  local active = lifecycle == 'playing' or lifecycle == 'results' or lifecycle == 'set_complete'
  if not active then return {rank=nil,accuracy=nil} end
  if participant.validity == 'dnf' then return {rank='DNF',accuracy='—',tone='red'} end
  if participant.validity == 'invalid' then return {rank='INVALID',accuracy='—',tone='red'} end
  local rank = participant.rank and ('#'..tostring(participant.rank)) or '—'
  local accuracy = participant.accuracy ~= nil and string.format('%.2f%%',participant.accuracy) or '—'
  return {rank=rank,accuracy=accuracy,tone='white'}
end

function Dashboard.canBroadcast(context)
  if context.isHost then return true,'host' end
  local me = context.me
  if me and me.admitted and me.role == 'spectator' and me.commentatorAccess then
    return true,'commentator'
  end
  return false,nil
end

function Dashboard.scroll(selection, offset, count, delta, pageSize)
  pageSize = pageSize or 8
  if count <= 0 then return 1,0 end
  selection = math.max(1,math.min(count,(selection or 1)+(delta or 0)))
  offset = math.max(0,math.min(math.max(0,count-pageSize),offset or 0))
  if selection <= offset then offset = selection-1 end
  if selection > offset+pageSize then offset = selection-pageSize end
  return selection,offset
end

function Dashboard.nextFocus(focus,direction,inRoom,secondaryCount)
  secondaryCount=secondaryCount or 0
  if focus=='roster' and direction=='right' then return 'primary' end
  if focus=='primary' and direction=='left' and inRoom then return 'roster' end
  if focus=='primary' and direction=='up' then return 'help' end
  if focus=='primary' and direction=='down' then return secondaryCount>0 and 'secondary' or 'utility' end
  if focus=='utility' and direction=='up' then return inRoom and 'roster' or 'primary' end
  if focus=='help' and direction=='left' then return inRoom and 'roster' or 'primary' end
  return focus
end

local function action(id,label,description,tone,enabled)
  return {id=id,label=label,description=description,tone=tone or 'white',enabled=enabled ~= false}
end

function Dashboard.primary(context)
  local phase = Dashboard.phase(context)
  local current = activeRoom(context)
  local me = context.me
  local isHost = context.isHost == true
  local summary = Dashboard.summary(context)
  local chartVerified = context.chartVerified == true
  -- A participant-level result is authoritative even when it is explicitly
  -- false; this prevents a stale convenience flag from hiding a mismatch.
  if me and me.verified ~= nil then chartVerified = me.verified == true end

  if phase == 'runtime_starting' then
    return action('wait_runtime','STARTING ONLINE','Preparing room and broadcast services.','yellow',false)
  end
  if phase == 'runtime_error' then
    return action('open_installer','OPEN INSTALLER','Repair the local Online runtime and installed mod files.','yellow')
  end
  if phase == 'connect' then
    return action('host_room','HOST A ROOM','Create a password-protected direct-IP room.','green')
  end
  if current.lifecycle == 'countdown' or current.lifecycle == 'playing' then
    return action('race_locked','RACE IN PROGRESS','Room administration resumes after the chart.','cyan',false)
  end
  if me and me.admitted == false then
    return action('wait_approval','WAITING FOR APPROVAL','The password was accepted. The host must approve this request.','yellow',false)
  end
  if phase == 'results' then
    local index = current.currentSetlistIndex
    local hasNext = index ~= nil and index + 1 < #(current.setlist or {})
    if isHost and hasNext then
      return action('advance_set','NEXT CHART','Lock the next chart, locate it locally, and return players to verification.','green')
    end
    if isHost then
      return action('select_next_chart','SELECT NEXT CHART','Add the next chart or reorder the completed set.','cyan')
    end
    return action('view_results','VIEW RESULTS','Review standings, accuracy, and DNF outcomes.','cyan')
  end
  if not current.chart then
    if isHost then return action('select_chart','SELECT CHART','Choose the room chart or build a setlist.','cyan') end
    return action('wait_chart','WAITING FOR CHART','The host is choosing the next chart.','white',false)
  end
  if me and me.role == 'spectator' then
    if isHost and summary.allReady then
      return action('start_race','START RACE','Schedule the synchronized start for every ready player.','green')
    end
    if isHost then
      return action('wait_players','WAITING '..summary.ready..'/'..summary.players,'Directing this race. Start becomes available when every player is verified and ready.','white',false)
    end
    return action('watch_room','SPECTATING ROOM','Rankings and room state update live.','cyan',false)
  end
  if me and me.role ~= 'spectator' and not chartVerified then
    local hostCanTransfer=not isHost
      and current.allowChartTransfers~=false
      and current.chart.official~=true
      and current.chart.transferMode=='host_transfer'
    if hostCanTransfer then
      return action('request_chart','REQUEST HOST','Ask the host for the exact locked package. You still approve the offer before download.','yellow')
    end
    return action('locate_chart','FIND MATCHING CHART','Select the exact chart package and variant locked by the host.','yellow')
  end
  if me and not me.ready then
    return action('ready','READY','Confirm this verified chart and wait for the synchronized start.','green',chartVerified)
  end
  if isHost and summary.allReady then
    return action('start_race','START RACE','Schedule the synchronized start for every ready player.','green')
  end
  if isHost then
    return action('wait_players','WAITING '..summary.ready..'/'..summary.players,'Start becomes available when every player is verified and ready.','white',false)
  end
  return action('ready_locked','YOU ARE READY','Waiting for the host to schedule the race.','green',false)
end

function Dashboard.help(context, overlay)
  if overlay == 'setlist' then return 'SETLIST','Choose the active chart, arrange an ordered set, and advance after results.' end
  if overlay == 'broadcast' then return 'BROADCAST','Assign Players to Stream A-D. The featured stream drives delayed video, audio, and text exports.' end
  if overlay == 'history' then return 'MATCH HISTORY','Review saved room results. Raw event journals can be pruned independently from summaries.' end
  if overlay == 'settings' then return 'SETTINGS','Control the gameplay HUD and inspect runtime, network, renderer, and local API status.' end
  local phase = Dashboard.phase(context)
  if phase == 'connect' then return 'PLAY ONLINE','Host a direct-IP room or join using the host IP, UDP port, and room password.' end
  if phase == 'results' then return 'RESULTS','Review rankings and set totals. The host advances when another setlist chart remains.' end
  if phase == 'runtime_error' then return 'ONLINE REPAIR','Open the installer to validate the runtime, adapter, Lovely files, and firewall component.' end
  return 'ROOM CONTROL','Verify the locked chart, ready up, and follow the host-scheduled start. Select a player for details.'
end

return Dashboard
