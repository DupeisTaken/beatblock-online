use crate::model::GameplayState;
use anyhow::Result;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

pub fn write_exports(directory: &Path, state: &GameplayState) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    atomic(directory.join("player_name.txt"), &state.player_name)?;
    atomic(directory.join("song_name.txt"), &state.song_name)?;
    atomic(
        directory.join("accuracy.txt"),
        &format!("{:.2}%", state.accuracy),
    )?;
    atomic(directory.join("combo.txt"), &state.combo.to_string())?;
    atomic(directory.join("misses.txt"), &state.misses.to_string())?;
    atomic(directory.join("rank.txt"), &state.rank.to_string())?;
    atomic(directory.join("lobby_name.txt"), &state.lobby_name)?;
    atomic(
        directory.join("state.json"),
        &serde_json::to_string_pretty(state)?,
    )?;
    Ok(())
}

fn atomic(path: PathBuf, content: &str) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    replace_file(&temporary, &path)?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(source, destination)?;
    Ok(())
}
