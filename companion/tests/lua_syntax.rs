#![cfg(windows)]

use std::{
    ffi::{c_char, c_int, c_void, CStr},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::FreeLibrary,
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

type LuaNewState = unsafe extern "C" fn() -> *mut c_void;
type LuaLoadBuffer =
    unsafe extern "C" fn(*mut c_void, *const c_char, usize, *const c_char) -> c_int;
type LuaToString = unsafe extern "C" fn(*mut c_void, c_int, *mut usize) -> *const c_char;
type LuaClose = unsafe extern "C" fn(*mut c_void);

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn every_distributed_lua_chunk_compiles_with_beatblocks_lua_runtime() {
    let library_path = workspace().join(".reference/Beatblock/lua51.dll");
    let wide = library_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let library = unsafe { LoadLibraryW(wide.as_ptr()) };
    assert_ne!(
        library,
        std::ptr::null_mut(),
        "load {}",
        library_path.display()
    );

    let new_state: LuaNewState = unsafe { symbol(library, b"luaL_newstate\0") };
    let load_buffer: LuaLoadBuffer = unsafe { symbol(library, b"luaL_loadbuffer\0") };
    let to_string: LuaToString = unsafe { symbol(library, b"lua_tolstring\0") };
    let close: LuaClose = unsafe { symbol(library, b"lua_close\0") };
    let lua = unsafe { new_state() };
    assert!(!lua.is_null());

    let files = [
        "mod/shared/bbt/core.lua",
        "mod/shared/bbt/dashboard_model.lua",
        "mod/shared/bbt/online_state.lua",
        "mod/shared/bbt/ipc_thread.lua",
        "mod/shared/bbt/renderer.lua",
        "mod/standalone/lovely/bootstrap.toml",
        "mod/beatblock-plus/main.lua",
        "mod/beatblock-plus/config.lua",
        "mod/beatblock-plus/states/Online.lua",
    ];
    for relative in files {
        if relative.ends_with(".toml") {
            continue;
        }
        compile(lua, load_buffer, to_string, &workspace().join(relative));
    }

    unsafe {
        close(lua);
        FreeLibrary(library);
    }
}

fn compile(lua: *mut c_void, load_buffer: LuaLoadBuffer, to_string: LuaToString, path: &Path) {
    let source = std::fs::read(path).unwrap();
    let name = format!("@{}\0", path.display());
    let status = unsafe {
        load_buffer(
            lua,
            source.as_ptr().cast(),
            source.len(),
            name.as_ptr().cast(),
        )
    };
    if status != 0 {
        let error = unsafe {
            let pointer = to_string(lua, -1, std::ptr::null_mut());
            if pointer.is_null() {
                "unknown Lua compilation error".into()
            } else {
                CStr::from_ptr(pointer).to_string_lossy().into_owned()
            }
        };
        panic!("{} did not compile: {error}", path.display());
    }
}

unsafe fn symbol<T>(library: *mut c_void, name: &[u8]) -> T {
    let pointer = unsafe { GetProcAddress(library, name.as_ptr()) };
    assert!(pointer.is_some(), "missing DLL symbol");
    unsafe { std::mem::transmute_copy(&pointer) }
}
