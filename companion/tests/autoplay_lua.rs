#![cfg(windows)]

// Executes the renderer's pure autoplay decisions with Beatblock's own Lua 5.1
// runtime. Native note classes retain ownership of scoring and sound playback;
// this test locks the perfect/avoid decisions and their one-shot guard inputs.
use std::{
    ffi::{c_char, c_int, c_void, CStr},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::FreeLibrary,
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

type LuaNewState = unsafe extern "C" fn() -> *mut c_void;
type LuaOpenLibs = unsafe extern "C" fn(*mut c_void);
type LuaLoadBuffer =
    unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const c_char) -> c_int;
type LuaPCall = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int;
type LuaToString = unsafe extern "C" fn(*mut c_void, c_int, *mut usize) -> *const c_char;
type LuaClose = unsafe extern "C" fn(*mut c_void);

#[test]
fn canonical_autoplay_uses_native_one_shot_guards_and_avoids_hazards() {
    let renderer = include_str!("../../mod/shared/bbt/renderer.lua");
    let scenarios = r#"
Block={checkTouchingPaddle=function() return false,true end}
MineHold={checkTouchingPaddle=function() return true end}
local ordinaryProfile={options={
  accessibility={taps='default',sides='default'},
  audio={hitsounds=false,sfxvolume=0,musicvolume=0}
}}
savedata=ordinaryProfile
Renderer.autoplay=false
Renderer.installAutoplayHooks()
assert(ordinaryProfile.options.accessibility.taps=='default')
assert(ordinaryProfile.options.accessibility.sides=='default')
assert(ordinaryProfile.options.audio.hitsounds==false)
assert(ordinaryProfile.options.audio.sfxvolume==0)
assert(ordinaryProfile.options.audio.musicvolume==0)

local autoplayProfile={options={
  accessibility={taps='default',sides='default'},
  audio={hitsounds=false,sfxvolume=0,musicvolume=0}
}}
savedata=autoplayProfile
local saveCalls,jsonSaveCalls=0,0
sdfunc={save=function() saveCalls=saveCalls+1 end}
dpf={saveJson=function() jsonSaveCalls=jsonSaveCalls+1 end}
Renderer.autoplay=true
Renderer.autoplayHooksInstalled=false
Renderer.disableProfilePersistence()
Renderer.installAutoplayHooks()
sdfunc.save()
dpf.saveJson()
assert(saveCalls==0 and jsonSaveCalls==0)
assert(savedata.options.accessibility.taps=='auto')
assert(savedata.options.accessibility.sides=='auto')
assert(savedata.options.audio.hitsounds==true)
assert(savedata.options.audio.sfxvolume>0)
assert(savedata.options.audio.musicvolume>0)
assert(ordinaryProfile.options.accessibility.taps=='default')
assert(ordinaryProfile.options.accessibility.sides=='default')
assert(ordinaryProfile.options.audio.hitsounds==false)
assert(ordinaryProfile.options.audio.sfxvolume==0)
assert(ordinaryProfile.options.audio.musicvolume==0)

local function positiveCollision(name)
  local note={name=name,hitYet=false}
  local nativeHits=0
  for _=1,3 do
    local hit,barely=Block.checkTouchingPaddle(note)
    assert(hit and not barely)
    if hit and not note.hitYet then
      note.hitYet=true
      nativeHits=nativeHits+1
    end
  end
  assert(nativeHits==1,name..' produced a duplicate native hit')
end

positiveCollision('block')
positiveCollision('bounce')

local hold={name='hold',hitYet=false,reachedEnd=false}
local holdStartHits,holdEndHits=0,0
for _=1,3 do
  local hit,barely=Block.checkTouchingPaddle(hold)
  assert(hit and not barely)
  if hit and not hold.hitYet then hold.hitYet=true; holdStartHits=holdStartHits+1 end
  if hit and not hold.reachedEnd then hold.reachedEnd=true; holdEndHits=holdEndHits+1 end
end
assert(holdStartHits==1 and holdEndHits==1)

local side={sideHitYet=false,hitsoundPlayed=false}
local sideHits=0
for _=1,3 do
  if savedata.options.accessibility.sides=='auto' then side.sideHitYet=true end
  if side.sideHitYet and not side.hitsoundPlayed then
    side.hitsoundPlayed=true
    sideHits=sideHits+1
  end
end
assert(sideHits==1)

local extraTap={hitYet=false}
local extraTapHits=0
for _=1,3 do
  local tapHit=savedata.options.accessibility.taps=='auto'
  if tapHit and not extraTap.hitYet then
    extraTap.hitYet=true
    extraTapHits=extraTapHits+1
  end
end
assert(extraTapHits==1)

local mineHit,mineBarely=Block.checkTouchingPaddle({name='mine'})
assert(not mineHit and not mineBarely)
local mineHoldHit=MineHold.checkTouchingPaddle({name='mineHold'})
assert(not mineHoldHit)
"#;
    execute(&format!(
        r#"
local originalGetenv=os.getenv
os.getenv=function(name)
  if name=='BBT_RENDERER_AUTOPLAY' then return '1' end
  if name=='BBT_RENDERER_STREAM' then return 'AUTOPLAY' end
  if name=='BBT_RENDERER_AUDIO' then return '1' end
  if name=='BBT_RENDERER_STATE_PATH' then return 'autoplay.bbtstate' end
  if name=='BBT_RENDERER_ERROR_PATH' then return 'autoplay.bbterror' end
  return originalGetenv(name)
end
local Renderer=(function()
{renderer}
end)()
os.getenv=originalGetenv
assert(Renderer.decodeHexUtf8('437573746f6d204c6576656c732f323465656576302defbfa52f')
  == 'Custom Levels/24eeev0-\239\191\165/')
assert(Renderer.decodeHexUtf8('e8b685e7baa7e99abee5baa6') == '\232\182\133\231\186\167\233\154\190\229\186\166')
assert(Renderer.decodeHexUtf8('not-hex') == nil)
assert(Renderer.decodeHexUtf8('abc') == nil)
os.getenv=function(name)
  if name=='BBT_RENDERER_CHART_HEX' then
    return '437573746f6d204c6576656c732f323465656576302defbfa52f'
  end
  if name=='BBT_RENDERER_CHART' then return 'lossy-legacy-value' end
  if name=='BBT_RENDERER_VARIANT' then return 'ASCII fallback' end
end
assert(Renderer.readUtf8Environment('BBT_RENDERER_CHART_HEX', 'BBT_RENDERER_CHART')
  == 'Custom Levels/24eeev0-\239\191\165/')
assert(Renderer.readUtf8Environment('BBT_RENDERER_VARIANT_HEX', 'BBT_RENDERER_VARIANT')
  == 'ASCII fallback')
os.getenv=originalGetenv
{scenarios}
"#
    ));
}

#[test]
fn room_modifiers_override_gameplay_without_persisting_local_preferences() {
    let core = include_str!("../../mod/shared/bbt/core.lua");
    let online = include_str!("../../mod/shared/bbt/online_state.lua");
    let scenarios = r#"
local localPreferences={vfx='none',taps='auto',sides='auto',barelies='lenient'}
savedata={options={accessibility=localPreferences}}
local persisted={}
local nativeSave=function(marker)
  persisted={
    vfx=savedata.options.accessibility.vfx,
    taps=savedata.options.accessibility.taps,
    sides=savedata.options.accessibility.sides,
    barelies=savedata.options.accessibility.barelies,
  }
  return 'saved-'..tostring(marker)
end
sdfunc={save=nativeSave}

local policy,modifierError=BBT.applyRoomModifiers({modifiers={
  rate=1.7,vfx='decreased',taps='strict',sides='lenient',barelies='strict',restartOn='miss'
}})
assert(policy and not modifierError)
assert(policy.rate==1.7 and policy.restartOn=='miss')
assert(savedata.options.accessibility.vfx=='decreased')
assert(savedata.options.accessibility.taps=='strict')
assert(savedata.options.accessibility.sides=='lenient')
assert(savedata.options.accessibility.barelies=='strict')

assert(sdfunc.save('preferences')=='saved-preferences')
assert(persisted.vfx=='none' and persisted.taps=='auto')
assert(persisted.sides=='auto' and persisted.barelies=='lenient')
assert(savedata.options.accessibility.vfx=='decreased')
assert(savedata.options.accessibility.taps=='strict')

local previousGameState={leave=function() end}
cs=previousGameState
bs={load=function(name)
  assert(name=='Game')
  return {init=function(self) self.initialized=true end}
end}
GameManager={transferStateData=function(_,nextState)
  nextState.rateMod=4.0
  nextState.restartOn='barely'
end}
BBT.lastLobby={id='room',lifecycle='countdown',modifiers={
  rate=1.7,vfx='decreased',taps='strict',sides='lenient',barelies='strict',restartOn='miss'
}}
BBT.localChart={levelPath='levels/test/',variantInfo={},levelData={},soundData={}}
BBT.chartVerified=true
BBT.scheduledStartTimeMs=0
BBT.launching=false
assert(BBT.maybeLaunchScheduledChart())
assert(cs.initialized==true)
assert(cs.rateMod==1.7 and cs.restartOn=='miss')

    package.preload['bbt.dashboard_model']=function() return {} end
    package.preload['bbt.dashboard_components']=function()
      return {new=function() return {} end}
    end
    Gamestate={new=function(_,name)
      local state={name=name}
      function state:setInit(callback) self.init=callback end
      function state:setUpdate(callback) self.updateState=callback end
      function state:setBgDraw(callback) self.bgDrawState=callback end
      function state:setFgDraw(callback) self.drawState=callback end
      return state
    end}
    love={graphics={setFont=function() end},keyboard={setTextInput=function() end}}
    fonts={digitalDisco={}}
    shuv={pal={},resetPal=function() end,showBadColors=true}
    em={clear=function() end}
    mouse={disableGameplay=function() end}
    BBT.lastLobby={id='room',lifecycle='results',participants={}}
    BBT.startOnlineRuntime=function() end
    BBT.command=function() return 'snapshot-request' end
    local online=(function()
{online}
    end)()()
    online:init()
    assert(sdfunc.save==nativeSave)
assert(savedata.options.accessibility.vfx=='none')
assert(savedata.options.accessibility.taps=='auto')
assert(savedata.options.accessibility.sides=='auto')
assert(savedata.options.accessibility.barelies=='lenient')

local sanitized=BBT.roomModifierPolicy({modifiers={
  rate=1.25,vfx='forged',taps='forged',sides='forged',barelies='forged',restartOn='forged'
}})
assert(sanitized.rate==1.0 and sanitized.vfx=='full' and sanitized.taps=='default')
assert(sanitized.sides=='default' and sanitized.barelies=='default' and sanitized.restartOn=='none')
"#;
    let scenarios = scenarios.replace("{online}", online);
    execute(&format!(
        r#"
local BBT=(function()
{core}
end)()
{scenarios}
"#
    ));
}

fn execute(source: &str) {
    let library_path = std::env::var_os("BBT_UI_FIXTURE")
        .or_else(|| std::env::var_os("BBT_GAME_FIXTURE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.test/Beatblock"))
        .join("lua51.dll");
    let wide = library_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let library = unsafe { LoadLibraryW(wide.as_ptr()) };
    assert!(!library.is_null(), "load {}", library_path.display());
    let new_state: LuaNewState = unsafe { symbol(library, b"luaL_newstate\0") };
    let open_libs: LuaOpenLibs = unsafe { symbol(library, b"luaL_openlibs\0") };
    let load_buffer: LuaLoadBuffer = unsafe { symbol(library, b"luaL_loadbuffer\0") };
    let pcall: LuaPCall = unsafe { symbol(library, b"lua_pcall\0") };
    let to_string: LuaToString = unsafe { symbol(library, b"lua_tolstring\0") };
    let close: LuaClose = unsafe { symbol(library, b"lua_close\0") };
    let lua = unsafe { new_state() };
    assert!(!lua.is_null());
    unsafe { open_libs(lua) };
    let name = b"@autoplay_renderer_test\0";
    let load = unsafe {
        load_buffer(
            lua,
            source.as_ptr().cast(),
            source.len(),
            name.as_ptr().cast(),
        )
    };
    if load != 0 {
        panic!(
            "autoplay renderer did not compile: {}",
            error(lua, to_string)
        );
    }
    let status = unsafe { pcall(lua, 0, 0, 0) };
    if status != 0 {
        panic!(
            "autoplay renderer scenario failed: {}",
            error(lua, to_string)
        );
    }
    unsafe {
        close(lua);
        FreeLibrary(library);
    }
}

fn error(lua: *mut c_void, to_string: LuaToString) -> String {
    unsafe {
        let pointer = to_string(lua, -1, std::ptr::null_mut());
        if pointer.is_null() {
            "unknown Lua error".into()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    }
}

unsafe fn symbol<T>(library: *mut c_void, name: &[u8]) -> T {
    let pointer = unsafe { GetProcAddress(library, name.as_ptr()) };
    assert!(pointer.is_some(), "missing DLL symbol");
    unsafe { std::mem::transmute_copy(&pointer) }
}
