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

function Components.new(palette)
  local ui = {palette=palette, audit={controls={},text={},issues={}}}

  function ui:begin()
    self.audit = {controls={},text={},issues={}}
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
    self.audit.text[#self.audit.text+1] = {value=value,x=x,y=y,w=width,h=font:getHeight()}
    if x < 0 or y < 0 or x + width > 600 or y + font:getHeight() > 360 then
      self.audit.issues[#self.audit.issues+1] = 'text_outside_canvas:'..value
    end
    return value
  end

  function ui:wrapped(value, x, y, width, maxLines, color)
    local font = love.graphics.getFont()
    local _, lines = font:getWrap(tostring(value or ''), width)
    maxLines = maxLines or #lines
    for index=1,math.min(maxLines,#lines) do
      local line = lines[index]
      if index == maxLines and #lines > maxLines then line = self:fit(line..'...',width,font) end
      self:text(line,x,y+(index-1)*(font:getHeight()+1),width,'left',color,font)
    end
  end

  function ui:panel(x, y, width, height, title)
    self:color('panel'); love.graphics.rectangle('fill',x,y,width,height,3,3)
    self:color('raised'); love.graphics.rectangle('line',x+.5,y+.5,width-1,height-1,3,3)
    if title then
      self:text(title,x+8,y+6,width-16,'left','muted')
      self:color('raised'); love.graphics.line(x+8,y+22,x+width-8,y+22)
    end
  end

  function ui:button(id, x, y, width, height, label, focused, color, enabled)
    enabled = enabled ~= false
    local fill = enabled and (focused and (self.palette[color or 'cyan']) or self.palette.raised) or self.palette.disabled
    setc(fill)
    love.graphics.rectangle('fill',x,y,width,height,2,2)
    if focused then
      self:color('white'); love.graphics.rectangle('line',x+.5,y+.5,width-1,height-1,2,2)
    end
    local font = love.graphics.getFont()
    local textY = y + math.floor((height-font:getHeight())/2)
    self:text(label,x+5,textY,width-10,'center',enabled and 'black' or 'dimBlack',font)
    self.audit.controls[#self.audit.controls+1] = {id=id,x=x,y=y,w=width,h=height,focused=focused}
    if height < 22 then self.audit.issues[#self.audit.issues+1] = 'undersized_control:'..tostring(id) end
  end

  function ui:chip(id, x, y, width, label, selected, color)
    self:color(selected and (color or 'cyan') or 'raised')
    love.graphics.rectangle(selected and 'fill' or 'line',x+.5,y+.5,width-1,21,2,2)
    local textColor = selected and 'black' or (color or 'white')
    self:text(label,x+3,y+5,width-6,'center',textColor)
    self.audit.controls[#self.audit.controls+1] = {id=id,x=x,y=y,w=width,h=22,focused=selected}
  end

  function ui:status(x, y, label, color)
    self:color(color or 'white'); love.graphics.circle('fill',x+3,y+6,2)
    self:text(label,x+9,y,120,'left',color or 'white')
  end

  function ui:issues()
    return self.audit.issues
  end

  return ui
end

return Components
