-- GPU/readback harness for the real renderer module. The raw gameplay canvas
-- deliberately uses the red palette-index color from the reported OBS bug,
-- while the shaded canvas uses an unmistakable final-view color.
package.path=package.path..';./?.lua;./?/init.lua'

project={res={x=600,y=360}}
savedata={options={accessibility={vfx='full'}}}
shaders={}
cs={
  name='Game',
  bgColor=6,
  bg={fixture=true},
  drawVideoBG=true,
  vfx={
    chromaticAberration={enabled=false},
    bgNoise=1,
    bgNoise_OLD={enable=true},
  },
}

local playerEntity={class={name='Player'},skipRender=false}
local noteEntity={class={name='Block'},skipRender=false}
local hitEntity={class={name='HitParticle'},skipRender=false}
local sceneryEntity={class={name='Deco'},skipRender=false}
cs.p=playerEntity
entities={playerEntity,noteEntity,hitEntity,sceneryEntity}

local Renderer=require('bbt.renderer')
local rawCanvas
local shadedCanvas
local frames=0
local startedAt

function love.load()
  love.window.setMode(320,180,{resizable=false,vsync=0})
  shaders.chromaticAberration=love.graphics.newShader([[
    vec4 effect(vec4 color, Image texture, vec2 textureCoords, vec2 screenCoords) {
      vec4 pixel=Texel(texture,textureCoords);
      return vec4(pixel.b,pixel.g,pixel.r,pixel.a)*color;
    }
  ]])
  cs.vfx.chromaticAberration.enabled=true
  Renderer.init()
  assert(Renderer.active,'renderer fixture did not initialize')
  Renderer.beginGameplayOnly(cs)
  assert(cs.bgColor==0 and cs.bg==nil and cs.drawVideoBG==false,'renderer retained a chart backdrop')
  assert(cs.vfx.bgNoise==0 and cs.vfx.bgNoise_OLD.enable==false,'renderer retained background noise')
  assert(not playerEntity.skipRender and not noteEntity.skipRender and not hitEntity.skipRender,
    'renderer hid gameplay entities')
  assert(sceneryEntity.skipRender,'renderer retained decorative scenery')
  Renderer.endGameplayOnly()
  assert(cs.bgColor==6 and cs.bg and cs.drawVideoBG==true,'renderer did not restore chart backdrop state')
  assert(cs.vfx.bgNoise==1 and cs.vfx.bgNoise_OLD.enable==true,'renderer did not restore background VFX')
  assert(sceneryEntity.skipRender==false,'renderer did not restore entity visibility')
  -- Use the production synchronous fallback for deterministic completion before
  -- the short-lived fixture exits; the QA LÖVE build predates readbackTexture,
  -- so adapt its equivalent Canvas:newImageData API at the harness boundary.
  love.graphics.readbackTextureAsync=nil
  love.graphics.readbackTexture=function(texture)
    local image=texture:newImageData()
    local ffi=require('ffi')
    return {
      getFFIPointer=function()
        if image.getFFIPointer then return image:getFFIPointer() end
        return ffi.cast('uint8_t*',image:getPointer())
      end,
      getSize=function() return image:getSize() end,
    }
  end
  rawCanvas=love.graphics.newCanvas(600,360,{format='rgba8',readable=true,dpiscale=1})
  shadedCanvas=love.graphics.newCanvas(600,360,{format='rgba8',readable=true,dpiscale=1})
  rawCanvas:renderTo(function()
    love.graphics.clear(1,0,0,1)
  end)
  shadedCanvas:renderTo(function()
    love.graphics.clear(.2,.4,.6,1)
  end)
  startedAt=love.timer.getTime()
end

function love.draw()
  Renderer.capturePlayerView(rawCanvas,shadedCanvas)
  frames=frames+1
  if frames>=45 and love.timer.getTime()-startedAt>=.75 then
    print(string.format(
      'active=%s sequence=%s capture=%s dropped=%s error=%s',
      tostring(Renderer.active),tostring(Renderer.sequence),
      tostring(Renderer.captureSequence),tostring(Renderer.droppedFrames),
      tostring(Renderer.captureError)
    ))
    Renderer.shutdown()
    love.event.quit(0)
  end
end
