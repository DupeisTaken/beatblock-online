-- Small measured component library for the fixed 600x360 Online canvas.
-- Geometry is recorded while drawing so the QA harness can fail on overflow,
-- overlap, or controls that are too small instead of relying on pixels alone.
local Components = {}

local function setc(color, alpha)
  love.graphics.setColor(color[1], color[2], color[3], alpha or color[4] or 1)
end

local function utf8Prefix(value, count)
  if utf8 and utf8.offset then
    local stop = utf8.offset(value, count + 1)
    return stop and value:sub(1, stop - 1) or value
  end
  return value:sub(1, count)
end

local function newAudit()
  return {controls={},text={},panels={},issues={},veil=0}
end

function Components.new(palette)
  local ui = {palette=palette, audit=newAudit()}

  function ui:begin()
    self.audit = newAudit()
  end

  function ui:issue(value)
    local issues = self.audit.issues
    issues[#issues+1] = value
  end

  function ui:color(name, alpha)
    setc(self.palette[name] or self.palette.white, alpha)
  end

  function ui:fit(value, width, font)
    value = tostring(value or '')
    font = font or love.graphics.getFont()
    if font:getWidth(value) <= width then return value end
    local suffix = '...'
    local low, high = 0, #value
    while low < high do
      local middle = math.ceil((low + high) / 2)
      if font:getWidth(utf8Prefix(value, middle)..suffix) <= width then low = middle else high = middle - 1 end
    end
    return utf8Prefix(value, low)..suffix
  end

  function ui:text(value, x, y, width, align, color, font)
    font = font or love.graphics.getFont()
    value = self:fit(value, width, font)
    self:color(color or 'white')
    love.graphics.printf(value, x, y, width, align or 'left')
    local height = font:getHeight()
    -- Record the inked run as well as the layout box. Sibling label/value
    -- columns deliberately share horizontal space, so only the glyphs that are
    -- actually painted can decide whether two blocks collide.
    local inkWidth = math.min(width, font:getWidth(value))
    local inkX = x
    if align == 'right' then inkX = x + width - inkWidth
    elseif align == 'center' then inkX = x + math.floor((width - inkWidth) / 2) end
    local entry = {value=value,x=x,y=y,w=width,h=height,ix=inkX,iw=inkWidth,panel=0}
    self.audit.text[#self.audit.text+1] = entry
    if x < 0 or y < 0 or x + width > 600 or y + height > 360 then
      self:issue('text_outside_canvas:'..value)
    end
    -- Attribute the block to the innermost panel it touches. Content that
    -- starts past a panel (navigation, header) belongs to no panel and is not
    -- constrained; content that starts inside one must stay inside it.
    local panels = self.audit.panels
    for index=#panels,1,-1 do
      local panel = panels[index]
      if x < panel.x + panel.w and panel.x < x + width
        and y < panel.y + panel.h and panel.y < y + height then
        entry.panel = index
        if x < panel.x or y < panel.y
          or x + width > panel.x + panel.w or y + height > panel.y + panel.h then
          self:issue('text_outside_panel:'..value)
        end
        break
      end
    end
    return value
  end

  function ui:wrapped(value, x, y, width, maxLines, color)
    local font = love.graphics.getFont()
    local _, lines = font:getWrap(tostring(value or ''), width)
    maxLines = maxLines or #lines
    -- Silently dropping copy is data loss, not a layout success. Budget the
    -- real wrapped height instead of letting an ellipsis hide the sentence.
    if #lines > maxLines then
      self:issue('text_wrap_overflow:'..tostring(#lines)..'>'..tostring(maxLines)..':'..tostring(lines[1] or ''))
    end
    for index=1,math.min(maxLines,#lines) do
      -- getWrap keeps the space it broke on. Measuring that space as ink makes a
      -- left-aligned column look wider than the glyphs it actually paints, which
      -- is enough on its own to report a neighbouring column as an overlap.
      local line = (lines[index]:gsub('%s+$',''))
      if index == maxLines and #lines > maxLines then line = self:fit(line..'...',width,font) end
      self:text(line,x,y+(index-1)*(font:getHeight()+1),width,'left',color,font)
    end
  end

  function ui:panel(x, y, width, height, title)
    self:color('panel'); love.graphics.rectangle('fill',x,y,width,height,3,3)
    self:color('raised'); love.graphics.rectangle('line',x+.5,y+.5,width-1,height-1,3,3)
    local panels = self.audit.panels
    panels[#panels+1] = {x=x,y=y,w=width,h=height}
    if title then
      self:text(title,x+8,y+3,width-16,'left','muted')
      self:color('raised'); love.graphics.line(x+8,y+22,x+width-8,y+22)
    end
  end

  -- An opaque focus veil avoids palette-invalid alpha blending and removes
  -- inactive controls from the visual hierarchy while a modal owns input.
  function ui:veil()
    self:color('black')
    love.graphics.rectangle('fill',0,0,600,360)
    -- Everything painted so far is hidden. Retire those panels and text blocks
    -- from the geometric passes so the modal is audited on its own.
    self.audit.panels = {}
    self.audit.veil = #self.audit.text
  end

  function ui:button(id, x, y, width, height, label, focused, color, enabled)
    enabled = enabled ~= false
    -- Text blocks that already exist are about to be painted over by this
    -- control's opaque fill. Remember the count so :finish can tell "drawn
    -- before, and therefore hidden" from this control's own label.
    local covered = #self.audit.text
    local fill = enabled and (focused and (self.palette[color or 'cyan']) or self.palette.raised) or self.palette.black
    setc(fill)
    love.graphics.rectangle('fill',x,y,width,height,2,2)
    if focused then
      self:color('white'); love.graphics.rectangle('line',x+.5,y+.5,width-1,height-1,2,2)
    elseif not enabled then
      self:color('muted'); love.graphics.rectangle('line',x+.5,y+.5,width-1,height-1,2,2)
    end
    local font = love.graphics.getFont()
    local textY = y + math.floor((height-font:getHeight())/2)
    if font:getWidth(tostring(label or '')) > width-10 then
      self:issue('button_label_overflow:'..tostring(id))
    end
    self:text(label,x+5,textY,width-10,'center',enabled and 'black' or 'muted',font)
    self.audit.controls[#self.audit.controls+1] = {id=id,x=x,y=y,w=width,h=height,focused=focused,cover=covered}
    if height < 22 then self:issue('undersized_control:'..tostring(id)) end
  end

  function ui:chip(id, x, y, width, label, selected, color, focused)
    local covered = #self.audit.text
    self:color(selected and (color or 'cyan') or 'raised')
    love.graphics.rectangle(selected and 'fill' or 'line',x+.5,y+.5,width-1,21,2,2)
    if focused then
      self:color('white')
      love.graphics.rectangle('line',x+2.5,y+2.5,width-5,17,2,2)
    end
    local textColor = selected and 'black' or (color or 'white')
    local font = love.graphics.getFont()
    self:text(label,x+3,y+math.floor((22-font:getHeight())/2),width-6,'center',textColor,font)
    self.audit.controls[#self.audit.controls+1] = {id=id,x=x,y=y,w=width,h=22,focused=focused==true,cover=covered}
  end

  function ui:status(x, y, label, color)
    self:color(color or 'white'); love.graphics.circle('fill',x+3,y+6,2)
    self:text(label,x+9,y,120,'left',color or 'white')
  end

  -- Deferred geometric pass. Text blocks are compared only against blocks that
  -- share a panel, so unrelated columns and veiled workspace content can never
  -- raise a false collision. Bounded by the small per-frame block count and
  -- allocation free, so the shipped mod can keep running it every frame.
  function ui:finish()
    local text = self.audit.text
    local count = #text
    for left=self.audit.veil+1,count do
      local a = text[left]
      if a.panel > 0 and a.iw > 0 then
        for right=left+1,count do
          local b = text[right]
          if b.panel == a.panel and b.iw > 0
            and a.y < b.y + b.h and b.y < a.y + a.h
            and a.ix < b.ix + b.iw and b.ix < a.ix + a.iw then
            self:issue('text_overlap:'..a.value..'|'..b.value)
          end
        end
      end
    end
    -- Controls paint an opaque rectangle, so copy that merely predates a button
    -- is clipped on screen while every text-versus-text pass still reports the
    -- frame as clean. Compare each control against the text it painted over.
    for _,control in ipairs(self.audit.controls) do
      local limit = math.min(control.cover or 0, count)
      for index=self.audit.veil+1,limit do
        local block = text[index]
        if block.iw > 0
          and block.ix < control.x + control.w and control.x < block.ix + block.iw
          and block.y < control.y + control.h and control.y < block.y + block.h then
          self:issue('text_behind_control:'..tostring(control.id)..':'..block.value)
        end
      end
    end
    return self.audit
  end

  function ui:issues()
    return self.audit.issues
  end

  return ui
end

return Components
