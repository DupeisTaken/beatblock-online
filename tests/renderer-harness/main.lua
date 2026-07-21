-- GPU/readback harness for the real renderer module. The raw gameplay canvas
-- deliberately uses the red palette-index color from the reported OBS bug,
-- while the shaded canvas uses an unmistakable final-view color.
package.path=package.path..';./?.lua;./?/init.lua'

project={res={x=600,y=360}}
savedata={options={accessibility={vfx='full'}},costumes={currentCostume='pirate'}}
shaders={}
shuv={usePalette=true}
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

local playerEntity={
  class={name='Player'},skipRender=false,
  x=300,y=180,radius=100,angle=0,anglePrevFrame=0,angleDelta=0,cumulativeAngle=0,
}
local noteEntity={class={name='Block'},skipRender=false}
local hitEntity={class={name='HitParticle'},skipRender=false}
local sceneryEntity={class={name='Deco'},skipRender=false}
cs.p=playerEntity
entities={playerEntity,noteEntity,hitEntity,sceneryEntity}
mouse={rx=0,ry=0,dx=0,dy=0,circleSnap='enabled'}

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
  shaders.palshader=love.graphics.newShader([[
    vec4 effect(vec4 color, Image texture, vec2 textureCoords, vec2 screenCoords) {
      vec4 pixel=Texel(texture,textureCoords);
      return vec4(pixel.g,pixel.r,pixel.b,pixel.a)*color;
    }
  ]])
  cs.vfx.chromaticAberration.enabled=true
  Renderer.init()
  assert(Renderer.active,'renderer fixture did not initialize')
  Renderer.useDefaultCostume()
  assert(savedata.costumes.currentCostume=='none',
    'renderer did not force Beatblock default Cranky costume')
  -- A graphics driver may never complete an async readback. Verify the public
  -- reclamation path frees both canvases and invalidates their old tickets.
  Renderer.readbackPending={true,true}
  Renderer.readbackRequests={{fixture=true},{fixture=true}}
  Renderer.readbackTickets={41,42}
  Renderer.readbackStartedAt={0,0}
  Renderer.reclaimStalledReadbacks(1.001)
  assert(not Renderer.readbackPending[1] and not Renderer.readbackPending[2],
    'renderer did not reclaim stalled readbacks')
  assert(Renderer.readbackTickets[1]==nil and Renderer.readbackTickets[2]==nil,
    'renderer retained abandoned readback tickets')
  Renderer.droppedFrames=0
  -- Native-faithful capture must never toggle chart scenery or VFX around a
  -- draw. Those objects are deliberately present in this fixture as sentinels.
  assert(cs.bgColor==6 and cs.bg and cs.drawVideoBG==true,'renderer changed chart backdrop state')
  assert(cs.vfx.bgNoise==1 and cs.vfx.bgNoise_OLD.enable==true,'renderer changed background VFX')
  assert(not playerEntity.skipRender and not noteEntity.skipRender and not hitEntity.skipRender
    and not sceneryEntity.skipRender,'renderer suppressed a native chart entity')
  -- The renderer enters Game directly instead of traversing SongSelect's grow
  -- callback. Menu:leave deliberately retains its animated background, so the
  -- direct handoff must explicitly release both entities and their old eases.
  local clearedEntities=0
  em={clear=function() clearedEntities=clearedEntities+1 end}
  flux={tweens={{fixture=1},{fixture=2}}}
  flux.remove=function(index) table.remove(flux.tweens,index) end
  Renderer.clearPreviousState()
  assert(clearedEntities==1 and #flux.tweens==0,
    'renderer retained pre-game menu entities or eases in the chart state')
  -- A Game-state early return is insufficient because Beatblock's outer loop
  -- updates flux and EntityManager afterward. The renderer must expose one
  -- shared freeze predicate for the Lovely wrapper around both systems.
  Renderer.hasInput=false
  Renderer.playing=false
  cs.startPending=false
  assert(Renderer.shouldFreezeSimulation(),
    'renderer did not freeze native eases and entities while awaiting input')
  cs.startPending=true
  assert(not Renderer.shouldFreezeSimulation(),
    'renderer froze the threaded chart preload before it completed')
  cs.startPending=false
  -- The hidden OS cursor sits in the upper-left. Verify that the remote vector
  -- is rebuilt in chart coordinates and only seeds native Player state once.
  Renderer.hasInput=true
  Renderer.angle=90
  Renderer.seedPaddle=true
  Renderer.steerPaddle()
  assert(math.abs(mouse.rx-400)<.001 and math.abs(mouse.ry-180)<.001,
    'renderer left the hidden cursor at the upper-left')
  assert(playerEntity.angle==90 and playerEntity.anglePrevFrame==90
    and playerEntity.angleDelta==0,'renderer did not seed the paddle consistently')
  playerEntity.angle=80
  Renderer.previousAngle=90
  Renderer.angle=180
  Renderer.lastInputSequence=2
  Renderer.steerPaddle()
  assert(playerEntity.angle==80,'renderer bypassed native paddle motion after its initial seed')
  assert(math.abs(mouse.rx-300)<.001 and math.abs(mouse.ry-280)<.001,
    'renderer did not refresh the remote mouse vector for the next native update')
  -- Player:update may cap the replayed cursor vector when a 60 Hz sample is
  -- repeated or skipped. The post-update boundary must restore the already
  -- capped source angle before higher-layer note collision runs.
  Renderer.applyPaddleState(playerEntity)
  assert(playerEntity.angle==180 and playerEntity.anglePrevFrame==90,
    'renderer did not apply the authoritative source paddle angle')
  assert(math.abs(playerEntity.angleDelta-90)<.001
    and math.abs(playerEntity.cumulativeAngle-180)<.001,
    'renderer did not preserve authoritative paddle motion state')
  Renderer.applyPaddleState(playerEntity)
  assert(playerEntity.angleDelta==0 and playerEntity.anglePrevFrame==180,
    'renderer reapplied angle delta for a repeated input sample')
  -- Native replay scoring can diverge after a skipped sample. The delayed
  -- source keyframe must win at the final post-GameManager boundary used by
  -- HUD rendering and the Results transition.
  Renderer.sourceAccuracy=98.75
  Renderer.sourceScore={
    hits=80,misses=1,barelies=1,combo=20,maxCombo=50,currentMaxHits=82,maxHits=100,
  }
  cs.hits=0; cs.misses=4; cs.barelies=0; cs.combo=0; cs.maxCombo=10
  cs.currentMaxHits=75; cs.maxHits=100
  Renderer.afterGameUpdate()
  assert(cs.hits==80 and cs.misses==1 and cs.barelies==1 and cs.combo==20
    and cs.maxCombo==50 and cs.currentMaxHits==82 and cs.maxHits==100,
    'renderer did not restore source-authored score state after native simulation')
  -- Read the final source keyframe through the production mmap path and verify
  -- that it alone authorizes Results, including the player's exact displayed
  -- accuracy and average timing offset.
  local gameplayState=cs
  GameManager={gradeCalc=function(_,pct)
    assert(pct==97.75,'renderer graded a locally reconstructed accuracy')
    return 's','none'
  end}
  function gameplayState:goToResults()
    assert(self.hits==97 and self.misses==2 and self.barelies==1 and self.maxCombo==75,
      'renderer entered Results before installing final source totals')
    self.results=true
    cs={name='Results'}
  end
  Renderer.update()
  assert(cs.name=='Results' and cs.hits==97 and cs.misses==2 and cs.barelies==1
    and cs.maxCombo==75 and cs.pctGrade==97.75 and cs.pctGradeRender=='97.75'
    and cs.offset==-10.25 and cs.lGrade=='s',
    'renderer Results did not come from the final player keyframe')
  cs=gameplayState
  local ffi=require('ffi')
  local scoreWords=ffi.cast('uint32_t*',Renderer.scores.pointer)
  scoreWords[0]=0
  Renderer.update()
  assert(Renderer.lastScoreSequence==0 and Renderer.sourceScore==nil
    and Renderer.sourceAccuracy==nil and not Renderer.resultsReady,
    'renderer retained source score state after the runtime reset its page')
  scoreWords[1]=0
  ffi.cast('float*',Renderer.scores.pointer+8)[0]=99.5
  ffi.cast('float*',Renderer.scores.pointer+12)[0]=2.5
  scoreWords[0]=1
  Renderer.update()
  assert(Renderer.lastScoreSequence==1 and Renderer.sourceAccuracy==99.5
    and Renderer.sourceOffset==2.5 and not Renderer.resultsReady,
    'renderer ignored a new run whose score commit counter restarted at one')
  Renderer.captureEnabled=true
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
  -- A chart can finish its native composition with a dither stencil and clip
  -- rectangle still active. The spectator copy must treat the already shaded
  -- canvas as final pixels, never reuse those masks on its output ring.
  love.graphics.stencil(function()
    love.graphics.rectangle('fill',0,0,32,32)
  end,'replace',1,true)
  love.graphics.setStencilTest('equal',1)
  love.graphics.setScissor(0,0,32,32)
  love.graphics.setColorMask(false,false,false,true)
  Renderer.capturePlayerView(rawCanvas,shadedCanvas)
  love.graphics.setColorMask(true,true,true,true)
  love.graphics.setScissor()
  love.graphics.setStencilTest()
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
