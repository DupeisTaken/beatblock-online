#![cfg(windows)]

// Executes the pure dashboard model with Beatblock's bundled Lua 5.1 runtime.
// These scenarios lock the user-facing action hierarchy independently of LÖVE.
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
fn adaptive_dashboard_prioritizes_the_next_role_aware_action() {
    let model = include_str!("../../mod/shared/bbt/dashboard_model.lua");
    let scenarios = r#"
local function player(name, role, admitted, verified, ready)
  return {sessionId=name,displayName=name,role=role,admitted=admitted,connected=true,
    verified=verified,ready=ready,accuracy=99.25,validity='pending'}
end
local function lobby(lifecycle, chart, participants, setlist, index)
  return {id='room-1',name='Room',lifecycle=lifecycle,chart=chart,participants=participants or {},
    setlist=setlist or {},currentSetlistIndex=index,hostSessionId='Host'}
end
local chart={songName='Signal',variant='Hard',official=false}
local host=player('Host','player',true,true,true)
local ready=player('Ready','player',true,true,true)
local waiting=player('Waiting','player',true,false,false)
local spectator=player('Caster','spectator',true,false,false)
local pending=player('Pending','player',false,false,false)

assert(Dashboard.phase({runtimeReady=false,runtimeStarting=true})=='runtime_starting')
assert(Dashboard.primary({runtimeReady=false,runtimeStarting=false}).id=='open_installer')
assert(Dashboard.primary({runtimeReady=true}).id=='host_room')
assert(Dashboard.primary({runtimeReady=true,room=lobby('forming',nil,{host}),me=host,isHost=true}).id=='select_chart')
assert(Dashboard.primary({runtimeReady=true,room=lobby('chart_locked',chart,{waiting}),me=waiting}).id=='locate_chart')
assert(Dashboard.primary({runtimeReady=true,room=lobby('chart_locked',chart,{waiting}),me=waiting,chartVerified=true}).id=='locate_chart')
waiting.verified=true
assert(Dashboard.primary({runtimeReady=true,room=lobby('chart_locked',chart,{waiting}),me=waiting,chartVerified=true}).id=='ready')
assert(Dashboard.primary({runtimeReady=true,room=lobby('ready',chart,{host,ready}),me=host,isHost=true}).id=='start_race')
assert(Dashboard.primary({runtimeReady=true,room=lobby('ready',chart,{host,waiting}),me=host,isHost=true}).id=='wait_players')
assert(Dashboard.primary({runtimeReady=true,room=lobby('playing',chart,{host}),me=host,isHost=true}).id=='race_locked')
assert(Dashboard.primary({runtimeReady=true,room=lobby('ready',chart,{spectator}),me=spectator}).id=='watch_room')
local set={{chart=chart},{chart=chart}}
assert(Dashboard.primary({runtimeReady=true,room=lobby('results',chart,{host},set,0),me=host,isHost=true}).id=='advance_set')
assert(Dashboard.primary({runtimeReady=true,room=lobby('set_complete',chart,{host},set,1),me=host,isHost=true}).id=='view_results')

local summary=Dashboard.summary({room=lobby('forming',chart,{host,ready,spectator,pending})})
assert(summary.players==2 and summary.spectators==1 and summary.pending==1)
assert(summary.ready==2 and summary.verified==2 and summary.allReady==true)
ready.connected=false
summary=Dashboard.summary({room=lobby('forming',chart,{host,ready,spectator,pending})})
assert(summary.players==1 and summary.ready==1 and summary.verified==1 and summary.allReady==true)
ready.connected=true
assert(Dashboard.participantStatus(pending)=='PENDING')
local selection,offset=Dashboard.scroll(8,0,16,1,8)
assert(selection==9 and offset==1)
selection,offset=Dashboard.scroll(16,8,16,-1,8)
assert(selection==15 and offset==8)
assert(Dashboard.nextFocus('primary','left',true,1)=='roster')
assert(Dashboard.nextFocus('primary','down',true,1)=='secondary')
assert(Dashboard.nextFocus('primary','down',false,0)=='utility')
assert(Dashboard.nextFocus('utility','up',true,1)=='roster')
local title,copy=Dashboard.help({runtimeReady=true},nil)
assert(title=='PLAY ONLINE' and string.find(copy,'direct%-IP'))

-- Participant selection is keyed by stable session id and survives a reorder
-- until the active filter intentionally excludes it.
local filteredContext={room=lobby('ready',chart,{spectator,pending,host,ready}),me=host,isHost=true}
local selected,visible=Dashboard.selectedParticipant(filteredContext,'all','Ready')
assert(selected.sessionId=='Ready' and #visible==4)
selected,visible=Dashboard.selectedParticipant(filteredContext,'players','Ready')
assert(selected.sessionId=='Ready' and #visible==2)
selected,visible=Dashboard.selectedParticipant(filteredContext,'pending','Ready')
assert(selected.sessionId=='Pending' and #visible==1)

assert(Dashboard.score(host,'ready').rank==nil)
local liveScore=Dashboard.score(host,'playing')
assert(liveScore.rank=='—' and liveScore.accuracy=='99.25%')
host.rank=1
assert(Dashboard.score(host,'results').rank=='#1')
host.validity='dnf'
assert(Dashboard.score(host,'results').rank=='DNF')
spectator.commentatorAccess=true
local allowed,authority=Dashboard.canBroadcast({me=spectator,isHost=false})
assert(allowed and authority=='commentator')
spectator.commentatorAccess=false
assert(Dashboard.canBroadcast({me=spectator,isHost=false})==false)
"#;
    execute(&format!(
        "local Dashboard=(function()\n{model}\nend)()\n{scenarios}"
    ));
}

fn execute(source: &str) {
    let library_path = std::env::var_os("BBT_UI_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"E:\beatblock-online\.test\ui-harness"))
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
    unsafe { open_libs(lua) };
    let name = b"@dashboard_model_test\0";
    let load = unsafe {
        load_buffer(
            lua,
            source.as_ptr().cast(),
            source.len(),
            name.as_ptr().cast(),
        )
    };
    if load != 0 {
        panic!("dashboard model did not compile: {}", error(lua, to_string));
    }
    let status = unsafe { pcall(lua, 0, 0, 0) };
    if status != 0 {
        panic!("dashboard model scenario failed: {}", error(lua, to_string));
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
