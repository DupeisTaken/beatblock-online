local COLORS = {
  background = { 0.025, 0.055, 0.066, 1 },
  grid = { 0.08, 0.18, 0.20, 0.42 },
  panel = { 0.045, 0.105, 0.12, 0.97 },
  panelRaised = { 0.065, 0.145, 0.16, 1 },
  border = { 0.16, 0.34, 0.37, 1 },
  text = { 0.92, 0.98, 0.98, 1 },
  muted = { 0.48, 0.64, 0.67, 1 },
  disabled = { 0.25, 0.34, 0.36, 1 },
  cyan = { 0.16, 0.84, 0.90, 1 },
  mint = { 0.43, 0.94, 0.64, 1 },
  amber = { 1.00, 0.74, 0.28, 1 },
  coral = { 1.00, 0.35, 0.34, 1 },
  ink = { 0.02, 0.07, 0.08, 1 },
}

local ACTION_X = 389
local ACTION_Y = 80
local ACTION_W = 193
local ACTION_H = 27
local ACTION_GAP = 5

local JOIN_COLUMNS = 8
local JOIN_KEY_W = 49
local JOIN_KEY_H = 24
local JOIN_KEY_GAP = 4
local JOIN_START_X = 88
local JOIN_START_Y = 139

local function setColor(value, alpha)
  love.graphics.setColor(value[1], value[2], value[3], alpha or value[4] or 1)
end

local function pointInRect(x, y, rect)
  return x >= rect.x and x <= rect.x + rect.w and y >= rect.y and y <= rect.y + rect.h
end

local function currentMouse()
  return mouse and mouse.rx or -1000, mouse and mouse.ry or -1000
end

local function pressed(control)
  if not maininput or not maininput.pressed then return false end
  local ok, value = pcall(maininput.pressed, maininput, control)
  return ok and value == true
end

local function confirmPressed()
  return pressed('select') or pressed('accept')
end

local function playUiSound(sound, volume)
  if te and sounds and sounds[sound] then
    pcall(te.play, sounds[sound], 'static', 'sfx', volume or 0.45)
  end
end

local function panel(x, y, w, h, raised)
  setColor(raised and COLORS.panelRaised or COLORS.panel)
  love.graphics.rectangle('fill', x, y, w, h, 3, 3)
  setColor(COLORS.border)
  love.graphics.rectangle('line', x + 0.5, y + 0.5, w - 1, h - 1, 3, 3)
end

local function menuState()
  local previous = cs
  local menuMusicManager = previous and previous.menuMusicManager
  cs = bs.load('Menu')
  if previous and previous.leave then previous:leave() end
  cs.menuMusicManager = menuMusicManager
  cs:init()
end

local function practiceState(onlineState)
  local previous = cs
  local menuMusicManager = onlineState and onlineState.menuMusicManager
  cs = bs.load('SongSelect')
  if previous and previous.leave then previous:leave() end
  if menuMusicManager then menuMusicManager:clearOnBeatHooks() end
  cs.menuMusicManager = menuMusicManager
  cs.topDirectory = 'Custom Levels/'
  cs.allowEditor = false
  cs:init()
end

local function currentLobby()
  return BBT and BBT.lastLobby and BBT.lastLobby.id ~= 'offline' and BBT.lastLobby or nil
end

local function playerCount(lobby, spectators)
  local count = 0
  for _, player in ipairs(lobby.players or {}) do
    if player.spectator == spectators then count = count + 1 end
  end
  return count
end

local function openJoin(self, spectator)
  self.joinMode = true
  self.joinSpectator = spectator == true
  self.joinSelection = 1
  self.joinCode = ''
  self.keyLatch = {}
  BBT.lastError = nil
end

local function clampSelection(self)
  if #self.actions == 0 then self.selection = 1; return end
  if self.selection < 1 then self.selection = #self.actions end
  if self.selection > #self.actions then self.selection = 1 end
end

local function action(id, label, description, enabled, run, disabledReason, tone)
  return {
    id = id,
    label = label,
    description = description,
    enabled = enabled ~= false,
    run = run,
    disabledReason = disabledReason,
    tone = tone or 'normal',
  }
end

local function buildActions(self)
  local lobby = currentLobby()
  local actions = {}
  local selectedId = self.actions and self.actions[self.selection] and self.actions[self.selection].id

  if not lobby or lobby.lifecycle == 'closed' then
    local canCreate = BBT.connected and (BBT.context.role == 'organizer' or BBT.context.role == 'operator')
    actions[#actions + 1] = action(
      'create',
      'CREATE PRIVATE LOBBY',
      'Open a code-protected competition room for up to 16 players.',
      canCreate,
      function()
        BBT.command('lobby.create_request', { name = (BBT.context.playerName or 'Player') .. "'s lobby" })
      end,
      not BBT.connected and 'Connect an invited account in the companion first.' or 'Your invited account needs the organizer role.',
      'primary'
    )
    actions[#actions + 1] = action(
      'join',
      'JOIN AS PLAYER',
      'Enter a lobby code and compete in the synchronized race.',
      BBT.connected,
      function() openJoin(self, false) end,
      'Connect an invited account in the companion first.'
    )
    actions[#actions + 1] = action(
      'spectate',
      'JOIN AS SPECTATOR',
      'Enter a lobby code to follow live rankings without occupying a player slot.',
      BBT.connected,
      function() openJoin(self, true) end,
      'Connect an invited account in the companion first.'
    )
    actions[#actions + 1] = action(
      'practice',
      'PRACTICE + TELEMETRY',
      'Play a custom chart locally while the companion and OBS receive telemetry.',
      true,
      function() practiceState(self) end,
      nil,
      'quiet'
    )
    actions[#actions + 1] = action('back', 'BACK TO MAIN MENU', 'Return to Beatblock.', true, menuState, nil, 'quiet')
  else
    local me = BBT.currentPlayer()
    local organizer = BBT.isOrganizer()
    local locked = lobby.lifecycle == 'countdown' or lobby.lifecycle == 'playing'

    if organizer and (lobby.lifecycle == 'forming' or lobby.lifecycle == 'chart_locked' or lobby.lifecycle == 'ready') then
      actions[#actions + 1] = action(
        'chart',
        lobby.chart and 'CHANGE CUSTOM CHART' or 'SELECT CUSTOM CHART',
        'Use Beatblock\'s song wheel to lock the exact chart package and variant.',
        not locked,
        function() BBT.openChartSelect('host') end,
        'The chart is locked while a race is starting or playing.',
        'primary'
      )
    end
    if lobby.chart and not BBT.chartVerified then
      actions[#actions + 1] = action(
        'verify',
        'LOCATE MATCHING CHART',
        'Select your local copy so the package and variant can be verified.',
        not locked,
        function() BBT.openChartSelect('verify') end,
        'Chart verification is locked during the race.',
        'primary'
      )
    end
    if lobby.chart and BBT.chartVerified and me and not me.spectator then
      actions[#actions + 1] = action(
        'ready',
        me.ready and 'CANCEL READY' or 'READY FOR RACE',
        me.ready and 'Unlock your setup to change the chart or leave.' or 'Confirm this chart, variant, build, and ruleset.',
        not locked and lobby.lifecycle ~= 'results',
        function() BBT.command('lobby.ready_request', { ready = not me.ready }) end,
        'Ready state cannot change during countdown, play, or results.',
        me.ready and 'quiet' or 'success'
      )
    end
    if organizer then
      local canStart = lobby.lifecycle == 'ready' and BBT.chartVerified and BBT.clockSynchronized
      local startReason = 'Every player must verify the chart and ready up.'
      if not BBT.clockSynchronized then startReason = 'Waiting for clock synchronization with the instance.' end
      if not BBT.chartVerified then startReason = 'Verify the selected chart on this client first.' end
      actions[#actions + 1] = action(
        'start',
        'START SYNCHRONIZED RACE',
        'Schedule the server-timed countdown for every ready client.',
        canStart,
        function() BBT.command('lobby.start_request') end,
        startReason,
        'success'
      )
      actions[#actions + 1] = action(
        'close',
        'CLOSE LOBBY',
        'Close this room for every player and spectator.',
        lobby.lifecycle ~= 'playing',
        function() BBT.command('lobby.close_request') end,
        'A lobby cannot be closed while the race is playing.',
        'danger'
      )
    else
      actions[#actions + 1] = action(
        'leave',
        'LEAVE LOBBY',
        'Return to the online home screen.',
        lobby.lifecycle ~= 'playing',
        function() BBT.command('lobby.leave_request') end,
        'A player cannot leave while the race is playing.',
        'danger'
      )
    end
    actions[#actions + 1] = action('back', 'BACK TO MAIN MENU', 'Leave this screen without leaving the lobby.', true, menuState, nil, 'quiet')
  end

  self.actions = actions
  if selectedId then
    for index, value in ipairs(actions) do
      if value.id == selectedId then self.selection = index; break end
    end
  end
  clampSelection(self)
end

local JOIN_KEYS = {}
local JOIN_CHARS = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789'
for index = 1, #JOIN_CHARS do JOIN_KEYS[#JOIN_KEYS + 1] = string.sub(JOIN_CHARS, index, index) end
JOIN_KEYS[#JOIN_KEYS + 1] = '<'
JOIN_KEYS[#JOIN_KEYS + 1] = 'JOIN'
JOIN_KEYS[#JOIN_KEYS + 1] = 'CANCEL'

local function joinKeyRect(index)
  local column = (index - 1) % JOIN_COLUMNS
  local row = math.floor((index - 1) / JOIN_COLUMNS)
  return {
    x = JOIN_START_X + column * (JOIN_KEY_W + JOIN_KEY_GAP),
    y = JOIN_START_Y + row * (JOIN_KEY_H + JOIN_KEY_GAP),
    w = JOIN_KEY_W,
    h = JOIN_KEY_H,
  }
end

local function submitJoin(self)
  if #self.joinCode < 6 then
    BBT.lastError = 'Lobby codes contain at least six characters.'
    return
  end
  BBT.command('lobby.join_request', { code = self.joinCode, spectator = self.joinSpectator })
  self.joinMode = false
end

local function useJoinKey(self, key)
  if key == '<' then
    self.joinCode = string.sub(self.joinCode, 1, math.max(0, #self.joinCode - 1))
  elseif key == 'CANCEL' then
    self.joinMode = false
  elseif key == 'JOIN' then
    submitJoin(self)
  elseif #self.joinCode < 8 then
    self.joinCode = self.joinCode .. key
    BBT.lastError = nil
  end
end

local function pollPhysicalJoinKeys(self)
  if not love.keyboard or not love.keyboard.isDown then return end
  for index = 1, #JOIN_CHARS do
    local character = string.sub(JOIN_CHARS, index, index)
    local key = string.lower(character)
    if character:match('%d') then key = character end
    local down = love.keyboard.isDown(key)
    if down and not self.keyLatch[key] then useJoinKey(self, character) end
    self.keyLatch[key] = down
  end
  local backspace = love.keyboard.isDown('backspace')
  if backspace and not self.keyLatch.backspace then useJoinKey(self, '<') end
  self.keyLatch.backspace = backspace
end

local function updateJoin(self)
  pollPhysicalJoinKeys(self)
  local oldSelection = self.joinSelection
  if pressed('menu_left') then self.joinSelection = self.joinSelection - 1 end
  if pressed('menu_right') then self.joinSelection = self.joinSelection + 1 end
  if pressed('menu_up') then self.joinSelection = self.joinSelection - JOIN_COLUMNS end
  if pressed('menu_down') then self.joinSelection = self.joinSelection + JOIN_COLUMNS end
  self.joinSelection = (self.joinSelection - 1) % #JOIN_KEYS + 1

  local mx, my = currentMouse()
  local hovered
  for index = 1, #JOIN_KEYS do
    if pointInRect(mx, my, joinKeyRect(index)) then hovered = index; break end
  end
  if hovered then self.joinSelection = hovered end
  if self.joinSelection ~= oldSelection then playUiSound('click', 0.3) end

  if pressed('back') or (mouse and mouse.altpress == -1) then self.joinMode = false; return end
  if love.keyboard and love.keyboard.isDown then
    local enter = love.keyboard.isDown('return') or love.keyboard.isDown('kpenter')
    if enter and not self.keyLatch.enter then
      submitJoin(self)
      self.keyLatch.enter = enter
      return
    end
    self.keyLatch.enter = enter
  end

  local clicked = hovered and mouse and mouse.pressed == 1
  if confirmPressed() or clicked then
    useJoinKey(self, JOIN_KEYS[self.joinSelection])
    playUiSound('hold', 0.35)
  end
end

local function drawBackground()
  setColor(COLORS.background)
  love.graphics.rectangle('fill', 0, 0, project.res.x, project.res.y)
  setColor(COLORS.grid)
  love.graphics.setLineWidth(1)
  for x = 0, project.res.x, 30 do love.graphics.line(x, 0, x, project.res.y) end
  for y = 0, project.res.y, 30 do love.graphics.line(0, y, project.res.x, y) end
  setColor(COLORS.cyan, 0.11)
  for x = 0, project.res.x, 18 do
    local height = 3 + ((x * 7) % 18)
    love.graphics.rectangle('fill', x, 51 - height, 8, height)
  end
end

local function connectionState()
  if BBT.connected then return 'INSTANCE ONLINE', COLORS.mint end
  if BBT.companionConnected then return 'INSTANCE OFFLINE', COLORS.amber end
  return 'COMPANION OFFLINE', COLORS.coral
end

local function drawHeader()
  setColor(COLORS.ink, 0.94)
  love.graphics.rectangle('fill', 0, 0, project.res.x, 53)
  setColor(COLORS.cyan)
  love.graphics.rectangle('fill', 0, 51, project.res.x, 2)

  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.cyan)
  love.graphics.print('BBT / ALPHA', 18, 9)
  love.graphics.setFont(fonts.main)
  setColor(COLORS.text)
  love.graphics.print('ONLINE COMPETITION', 18, 24)

  local label, tone = connectionState()
  local width = fonts.digitalDisco:getWidth(label) + 27
  setColor(COLORS.panelRaised)
  love.graphics.rectangle('fill', project.res.x - width - 17, 15, width, 23, 12, 12)
  setColor(tone)
  love.graphics.circle('fill', project.res.x - width - 6, 26.5, 3)
  love.graphics.setFont(fonts.digitalDisco)
  love.graphics.printf(label, project.res.x - width + 2, 21, width - 11, 'center')
end

local function drawStatusRow(y, label, value, tone)
  setColor(COLORS.border)
  love.graphics.line(35, y + 17, 350, y + 17)
  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.muted)
  love.graphics.print(label, 35, y)
  setColor(tone or COLORS.text)
  love.graphics.printf(value, 170, y, 180, 'right')
end

local function drawOnlineHome()
  panel(18, 68, 351, 226, false)
  love.graphics.setFont(fonts.main)
  setColor(COLORS.text)
  love.graphics.print('RACE CONTROL', 34, 84)
  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.muted)
  love.graphics.printf('Create a private lobby, join by code, or keep your OBS telemetry live during practice.', 34, 108, 315, 'left')

  local companionValue = BBT.companionConnected and 'CONNECTED' or 'NOT FOUND'
  local companionTone = BBT.companionConnected and COLORS.mint or COLORS.coral
  local instanceValue = BBT.connected and 'AUTHENTICATED' or 'SETUP REQUIRED'
  local instanceTone = BBT.connected and COLORS.mint or COLORS.amber
  drawStatusRow(151, 'LOCAL COMPANION', companionValue, companionTone)
  drawStatusRow(181, 'REMOTE INSTANCE', instanceValue, instanceTone)
  drawStatusRow(211, 'ACCOUNT ROLE', string.upper(BBT.context.role or 'NOT SIGNED IN'), BBT.context.role and COLORS.cyan or COLORS.muted)

  setColor(COLORS.panelRaised)
  love.graphics.rectangle('fill', 33, 250, 321, 27, 3, 3)
  setColor(COLORS.text)
  local guidance = BBT.connected and ('SIGNED IN AS  ' .. string.upper(BBT.context.playerName or 'PLAYER')) or 'OPEN THE COMPANION TO CONFIGURE AN INSTANCE + INVITE'
  love.graphics.printf(guidance, 40, 258, 307, 'center')
end

local function lifecycleTone(lifecycle)
  if lifecycle == 'ready' then return COLORS.mint end
  if lifecycle == 'countdown' or lifecycle == 'playing' then return COLORS.cyan end
  if lifecycle == 'results' then return COLORS.amber end
  return COLORS.text
end

local function drawPlayer(player, x, y, width, organizerId)
  local readyTone = player.ready and COLORS.mint or COLORS.muted
  if player.connected == false then readyTone = COLORS.coral end
  setColor(readyTone)
  love.graphics.rectangle('fill', x, y + 3, 4, 9, 1, 1)
  love.graphics.setFont(fonts.digitalDisco)
  setColor(player.connected == false and COLORS.muted or COLORS.text)
  local host = player.userId == organizerId and '*' or ''
  local name = host .. tostring(player.displayName or 'Player')
  if #name > 14 then name = string.sub(name, 1, 13) .. '>' end
  love.graphics.print(name, x + 9, y)
  local stat = player.rank and ('#' .. tostring(player.rank)) or (player.ready and 'READY' or 'WAIT')
  if tonumber(player.accuracy) then stat = string.format('%.2f', tonumber(player.accuracy)) end
  setColor(readyTone)
  love.graphics.printf(stat, x + width - 54, y, 54, 'right')
end

local function drawLobby(lobby)
  panel(18, 68, 351, 226, false)
  love.graphics.setFont(fonts.main)
  setColor(COLORS.text)
  local lobbyName = tostring(lobby.name or 'Competition lobby')
  if #lobbyName > 24 then lobbyName = string.sub(lobbyName, 1, 23) .. '>' end
  love.graphics.print(lobbyName, 33, 82)

  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.cyan)
  love.graphics.printf(tostring(lobby.code or '------'), 270, 84, 82, 'right')
  setColor(lifecycleTone(lobby.lifecycle))
  love.graphics.print(string.upper(lobby.lifecycle or 'forming'), 33, 105)
  setColor(COLORS.muted)
  love.graphics.printf(playerCount(lobby, false) .. '/16 PLAYERS  /  ' .. playerCount(lobby, true) .. ' WATCHING', 133, 105, 219, 'right')

  setColor(COLORS.panelRaised)
  love.graphics.rectangle('fill', 32, 126, 323, 42, 3, 3)
  love.graphics.setFont(fonts.digitalDisco)
  if lobby.chart then
    setColor(COLORS.text)
    local song = tostring(lobby.chart.songName or lobby.chart.packageName or 'Selected chart')
    if #song > 31 then song = string.sub(song, 1, 30) .. '>' end
    love.graphics.print(song, 42, 134)
    setColor(BBT.chartVerified and COLORS.mint or COLORS.amber)
    local verification = BBT.chartVerified and 'VERIFIED' or 'LOCAL COPY REQUIRED'
    love.graphics.print(verification, 42, 151)
    setColor(COLORS.muted)
    love.graphics.printf(tostring(lobby.chart.variant or 'Default'), 230, 151, 114, 'right')
  else
    setColor(COLORS.amber)
    love.graphics.print('WAITING FOR THE ORGANIZER TO LOCK A CHART', 42, 143)
  end

  setColor(COLORS.muted)
  love.graphics.print('ROSTER', 33, 177)
  if BBT.clockSynchronized then
    setColor(COLORS.cyan)
    love.graphics.printf('SYNC ' .. tostring(math.floor(BBT.clockRoundTripMs or 0)) .. 'ms', 264, 177, 88, 'right')
  end

  local competitors = {}
  for _, player in ipairs(lobby.players or {}) do
    if not player.spectator then competitors[#competitors + 1] = player end
  end
  for index = 1, math.min(16, #competitors) do
    local column = math.floor((index - 1) / 8)
    local row = (index - 1) % 8
    drawPlayer(competitors[index], 33 + column * 162, 195 + row * 11, 151, lobby.organizerId)
  end
end

local function actionRect(index)
  return { x = ACTION_X, y = ACTION_Y + (index - 1) * (ACTION_H + ACTION_GAP), w = ACTION_W, h = ACTION_H }
end

local function actionTone(value)
  if value.tone == 'danger' then return COLORS.coral end
  if value.tone == 'success' then return COLORS.mint end
  if value.tone == 'primary' then return COLORS.cyan end
  return COLORS.text
end

local function drawActions(self)
  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.muted)
  love.graphics.print('ACTIONS', ACTION_X, 65)
  for index, value in ipairs(self.actions) do
    local rect = actionRect(index)
    local selected = index == self.selection
    local tone = actionTone(value)
    if selected then
      setColor(value.enabled and tone or COLORS.disabled)
      love.graphics.rectangle('fill', rect.x, rect.y, rect.w, rect.h, 3, 3)
      setColor(COLORS.ink)
    else
      setColor(COLORS.panelRaised)
      love.graphics.rectangle('fill', rect.x, rect.y, rect.w, rect.h, 3, 3)
      setColor(value.enabled and tone or COLORS.disabled)
      love.graphics.rectangle('line', rect.x + 0.5, rect.y + 0.5, rect.w - 1, rect.h - 1, 3, 3)
      setColor(value.enabled and COLORS.text or COLORS.disabled)
    end
    love.graphics.printf(value.label, rect.x + 7, rect.y + 8, rect.w - 14, 'center')
  end
end

local function drawFooter(self)
  local selected = self.actions[self.selection]
  setColor(COLORS.ink, 0.97)
  love.graphics.rectangle('fill', 0, 304, project.res.x, 56)
  setColor(COLORS.border)
  love.graphics.line(0, 304, project.res.x, 304)
  love.graphics.setFont(fonts.digitalDisco)
  if selected then
    setColor(selected.enabled and actionTone(selected) or COLORS.amber)
    love.graphics.print(selected.enabled and 'READY' or 'LOCKED', 18, 314)
    setColor(COLORS.text)
    love.graphics.printf(selected.enabled and selected.description or selected.disabledReason or 'This action is unavailable.', 77, 314, 505, 'left')
  end
  setColor(COLORS.muted)
  love.graphics.printf('MOUSE / ARROWS  NAVIGATE     ENTER / A  SELECT     ESC / B  BACK', 18, 339, 564, 'center')
end

local function drawError()
  if not BBT.lastError then return end
  local message = tostring(BBT.lastError)
  if #message > 76 then message = string.sub(message, 1, 75) .. '>' end
  setColor(COLORS.coral)
  love.graphics.rectangle('fill', 18, 278, 564, 22, 3, 3)
  setColor(COLORS.ink)
  love.graphics.setFont(fonts.digitalDisco)
  love.graphics.printf(message, 25, 284, 550, 'center')
end

local function drawJoin(self)
  setColor(COLORS.ink, 0.80)
  love.graphics.rectangle('fill', 0, 53, project.res.x, project.res.y - 53)
  panel(47, 63, 506, 258, true)
  love.graphics.setFont(fonts.main)
  setColor(COLORS.text)
  love.graphics.printf(self.joinSpectator and 'JOIN AS SPECTATOR' or 'JOIN AS PLAYER', 47, 75, 506, 'center')
  love.graphics.setFont(fonts.digitalDisco)
  setColor(COLORS.muted)
  love.graphics.printf('TYPE THE LOBBY CODE OR USE THE GRID', 47, 99, 506, 'center')

  local display = self.joinCode .. string.rep('-', math.max(0, 6 - #self.joinCode))
  setColor(COLORS.ink)
  love.graphics.rectangle('fill', 170, 116, 260, 19, 3, 3)
  setColor(#self.joinCode >= 6 and COLORS.mint or COLORS.cyan)
  love.graphics.printf(display, 170, 120, 260, 'center')

  for index, key in ipairs(JOIN_KEYS) do
    local rect = joinKeyRect(index)
    local selected = index == self.joinSelection
    local special = key == 'JOIN' or key == 'CANCEL' or key == '<'
    local tone = key == 'JOIN' and COLORS.mint or (key == 'CANCEL' and COLORS.coral or COLORS.cyan)
    if selected then
      setColor(special and tone or COLORS.cyan)
      love.graphics.rectangle('fill', rect.x, rect.y, rect.w, rect.h, 2, 2)
      setColor(COLORS.ink)
    else
      setColor(COLORS.panel)
      love.graphics.rectangle('fill', rect.x, rect.y, rect.w, rect.h, 2, 2)
      setColor(special and tone or COLORS.border)
      love.graphics.rectangle('line', rect.x + 0.5, rect.y + 0.5, rect.w - 1, rect.h - 1, 2, 2)
      setColor(COLORS.text)
    end
    love.graphics.printf(key, rect.x, rect.y + 7, rect.w, 'center')
  end

  if BBT.lastError then
    setColor(COLORS.coral)
    love.graphics.printf(tostring(BBT.lastError), 67, 303, 466, 'center')
  else
    setColor(COLORS.muted)
    love.graphics.printf('6-8 CHARACTERS     BACKSPACE DELETE     ENTER JOIN', 67, 303, 466, 'center')
  end
end

local function activateAction(self, index)
  local value = self.actions[index]
  if not value then return end
  if not value.enabled then
    BBT.lastError = value.disabledReason or 'This action is currently unavailable.'
    playUiSound('click', 0.25)
    return
  end
  BBT.lastError = nil
  playUiSound('hold', 0.4)
  value.run()
end

local function updateActions(self)
  local oldSelection = self.selection
  if pressed('menu_up') then self.selection = self.selection - 1 end
  if pressed('menu_down') then self.selection = self.selection + 1 end
  if mouse and mouse.syInteger and mouse.syInteger ~= 0 then
    self.selection = self.selection - (mouse.syInteger > 0 and 1 or -1)
  end
  clampSelection(self)

  local mx, my = currentMouse()
  local hovered
  for index = 1, #self.actions do
    if pointInRect(mx, my, actionRect(index)) then hovered = index; break end
  end
  if hovered then self.selection = hovered end
  if self.selection ~= oldSelection then playUiSound('click', 0.28) end

  if hovered and mouse and mouse.pressed == 1 then
    activateAction(self, hovered)
  elseif confirmPressed() then
    activateAction(self, self.selection)
  elseif pressed('back') or (mouse and mouse.altpress == -1) then
    menuState()
  end
end

return function()
  local st = Gamestate:new('Online')

  st:setInit(function(self)
    em.clear({ self.menuMusicManager })
    mouse:disableGameplay()
    self.previousUsePalette = shuv.usePalette
    shuv.usePalette = false
    shuv.resetPal()
    love.graphics.setLineWidth(1)
    self.selection = 1
    self.actions = {}
    self.joinMode = false
    self.joinSpectator = false
    self.joinSelection = 1
    self.joinCode = ''
    self.keyLatch = {}
    buildActions(self)
  end)

  function st:leave()
    shuv.usePalette = self.previousUsePalette
    shuv.resetPal()
  end

  st:setUpdate(function(self, dt)
    if self.menuMusicManager then self.menuMusicManager:update(dt) end
    if BBT then BBT.update(dt) end
    if BBT and BBT.maybeLaunchScheduledChart() then return end
    if self.joinMode then updateJoin(self); return end
    buildActions(self)
    updateActions(self)
  end)

  st:setFgDraw(function(self)
    drawBackground()
    drawHeader()
    if currentLobby() then drawLobby(currentLobby()) else drawOnlineHome() end
    drawActions(self)
    drawError()
    drawFooter(self)
    if self.joinMode then drawJoin(self) end
    setColor(COLORS.text)
  end)

  return st
end
