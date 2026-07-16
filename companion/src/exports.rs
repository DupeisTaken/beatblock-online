use crate::model::{GameplayState, RendererSlot, RoomSnapshot};
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
        directory.join("gameplay.json"),
        &serde_json::to_string_pretty(state)?,
    )?;
    Ok(())
}

pub fn write_featured_exports(directory: &Path, state: &GameplayState) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    atomic(directory.join("featured_name.txt"), &state.player_name)?;
    atomic(
        directory.join("featured_accuracy.txt"),
        &format!("{:.2}%", state.accuracy),
    )?;
    atomic(
        directory.join("featured_combo.txt"),
        &state.combo.to_string(),
    )?;
    atomic(
        directory.join("featured_misses.txt"),
        &state.misses.to_string(),
    )?;
    atomic(directory.join("featured_rank.txt"), &state.rank.to_string())?;
    Ok(())
}

pub fn clear_featured_exports(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    for name in [
        "featured_name.txt",
        "featured_accuracy.txt",
        "featured_combo.txt",
        "featured_misses.txt",
        "featured_rank.txt",
    ] {
        atomic(directory.join(name), "")?;
    }
    Ok(())
}

pub fn write_room_exports(
    directory: &Path,
    room: &RoomSnapshot,
    slots: &[RendererSlot],
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    atomic(directory.join("room_name.txt"), &room.name)?;
    atomic(directory.join("lobby_name.txt"), &room.name)?;
    for slot in slots {
        let slot_directory = directory.join("streams").join(&slot.id);
        std::fs::create_dir_all(&slot_directory)?;
        atomic(
            slot_directory.join("state.json"),
            &serde_json::to_string_pretty(slot)?,
        )?;
        if let Some(participant_id) = slot.participant_id.as_deref() {
            if let Some(participant) = room
                .participants
                .iter()
                .find(|participant| participant.session_id == participant_id)
            {
                atomic(
                    slot_directory.join("player_name.txt"),
                    &participant.display_name,
                )?;
                atomic(
                    slot_directory.join("accuracy.txt"),
                    &format!("{:.2}%", participant.accuracy),
                )?;
                atomic(
                    slot_directory.join("combo.txt"),
                    &participant.totals.combo.to_string(),
                )?;
                atomic(
                    slot_directory.join("misses.txt"),
                    &participant.totals.misses.to_string(),
                )?;
                atomic(
                    slot_directory.join("rank.txt"),
                    &participant.rank.unwrap_or(0).to_string(),
                )?;
            }
        }
    }
    atomic(
        directory.join("state.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "room": room,
            "streams": slots,
        }))?,
    )?;
    Ok(())
}

pub fn atomic(path: PathBuf, content: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_gameplay_never_overwrites_featured_exports() {
        let root = std::env::temp_dir().join(format!("bbt-exports-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        atomic(root.join("featured_name.txt"), "Remote player").unwrap();
        let state = GameplayState {
            player_name: "Host".into(),
            song_name: "Song".into(),
            lobby_name: "Room".into(),
            ..GameplayState::default()
        };

        write_exports(&root, &state).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("featured_name.txt")).unwrap(),
            "Remote player"
        );
        assert!(root.join("gameplay.json").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clearing_featured_exports_removes_stale_values() {
        let root = std::env::temp_dir().join(format!("bbt-featured-{}", rand::random::<u64>()));
        write_featured_exports(
            &root,
            &GameplayState {
                player_name: "Player".into(),
                ..GameplayState::default()
            },
        )
        .unwrap();

        clear_featured_exports(&root).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("featured_name.txt")).unwrap(),
            ""
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
