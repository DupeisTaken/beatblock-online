use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const MAX_TRANSFER_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_ENTRY_NAME_BYTES: usize = 1_024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOffer {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub source_host: String,
    pub contains_executable_content: bool,
}

pub fn inspect_offer(path: &Path, source_host: &str) -> Result<TransferOffer> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_TRANSFER_BYTES {
        bail!("chart transfer must be a file no larger than 1 GiB");
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut executable = false;
    let mut archive = zip::ZipArchive::new(File::open(path)?)
        .context("transferred chart is not a valid ZIP archive")?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        bail!("chart package contains too many files");
    }
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let item = archive.by_index(index)?;
        validate_entry(&item)?;
        expanded = expanded.saturating_add(item.size());
        if expanded > MAX_TRANSFER_BYTES {
            bail!("expanded chart package exceeds 1 GiB");
        }
        executable |= executable_extension(Path::new(item.name()));
    }
    Ok(TransferOffer {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        size: metadata.len(),
        sha256: hex::encode(hasher.finalize()),
        source_host: source_host.to_owned(),
        contains_executable_content: executable,
    })
}

/// Materializes a selected chart directory as the archive transported over
/// QUIC. The archive remains outside Custom Levels and is reused by hash for
/// later setlist peers in the same room.
pub fn archive_chart_directory(source: &Path, destination: &Path) -> Result<PathBuf> {
    if source.is_file() {
        return Ok(source.to_path_buf());
    }
    if !source.is_dir() {
        bail!("selected custom chart package no longer exists");
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("partial");
    let file = File::create(&temporary)?;
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut count = 0usize;
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() || entry.path() == source {
            continue;
        }
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            bail!("chart package contains too many files");
        }
        let relative = entry.path().strip_prefix(source)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_dir() {
            archive.add_directory(format!("{name}/"), options)?;
            continue;
        }
        total = total.saturating_add(entry.metadata()?.len());
        if total > MAX_TRANSFER_BYTES {
            bail!("chart package exceeds the 1 GiB transfer limit");
        }
        archive.start_file(name, options)?;
        let mut input = File::open(entry.path())?;
        std::io::copy(&mut input, &mut archive)?;
    }
    archive.finish()?;
    if std::fs::metadata(&temporary)?.len() > MAX_TRANSFER_BYTES {
        let _ = std::fs::remove_file(&temporary);
        bail!("compressed chart package exceeds the 1 GiB transfer limit");
    }
    if destination.exists() {
        std::fs::remove_file(destination)?;
    }
    std::fs::rename(&temporary, destination)?;
    Ok(destination.to_path_buf())
}

pub fn install_received_package(
    archive_path: &Path,
    expected_hash: &str,
    imports_directory: &Path,
    executable_content_confirmed: bool,
) -> Result<PathBuf> {
    let offer = inspect_offer(archive_path, "room host")?;
    if !offer.sha256.eq_ignore_ascii_case(expected_hash) {
        bail!("transferred chart hash does not match the host offer");
    }
    if offer.contains_executable_content && !executable_content_confirmed {
        bail!(
            "this chart contains scripts or executable content and requires explicit confirmation"
        );
    }
    let destination = imports_directory.join(&offer.sha256);
    if destination.exists() {
        return Ok(destination);
    }
    std::fs::create_dir_all(imports_directory)?;
    let temporary = imports_directory.join(format!(".{}.partial", offer.sha256));
    if temporary.exists() {
        std::fs::remove_dir_all(&temporary)?;
    }
    std::fs::create_dir_all(&temporary)?;
    let result = extract_checked(archive_path, &temporary);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    std::fs::rename(&temporary, &destination)?;
    let mut receipt = File::create(destination.join(".bbt-import.json"))?;
    serde_json::to_writer_pretty(&mut receipt, &offer)?;
    receipt.write_all(b"\n")?;
    evict_cache(imports_directory, Some(&offer.sha256))?;
    Ok(destination)
}

/// Returns the on-disk size of BBT's isolated Online cache. Transferred charts
/// are never registered in the user's normal Custom Levels directory.
pub fn cache_size(imports_directory: &Path) -> u64 {
    directory_size(imports_directory).unwrap_or(0)
}

/// Clears every inactive managed package while protecting the chart currently
/// mounted by Online. Partial transfers are always safe to remove.
pub fn clear_cache(imports_directory: &Path, active_hash: Option<&str>) -> Result<u64> {
    if !imports_directory.exists() {
        return Ok(0);
    }
    let before = cache_size(imports_directory);
    for entry in std::fs::read_dir(imports_directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if active_hash.is_some_and(|active| active.eq_ignore_ascii_case(&name)) {
            continue;
        }
        if entry.path().is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(before.saturating_sub(cache_size(imports_directory)))
}

/// Enforces the 2 GiB LRU budget after an accepted transfer. Directory
/// modification time is the cache access clock; the active chart is pinned.
pub fn evict_cache(imports_directory: &Path, active_hash: Option<&str>) -> Result<()> {
    if cache_size(imports_directory) <= MAX_CACHE_BYTES {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(imports_directory)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !active_hash.is_some_and(|active| active.eq_ignore_ascii_case(&name))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for entry in entries {
        if cache_size(imports_directory) <= MAX_CACHE_BYTES {
            break;
        }
        std::fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

fn directory_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total = total.saturating_add(if metadata.is_dir() {
            directory_size(&entry.path())?
        } else {
            metadata.len()
        });
    }
    Ok(total)
}

fn extract_checked(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(archive_path)?)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        bail!("chart package contains too many files");
    }
    let canonical_destination = std::fs::canonicalize(destination)?;
    let mut expanded = 0u64;
    let mut copied_total = 0u64;
    for index in 0..archive.len() {
        let item = archive.by_index(index)?;
        validate_entry(&item)?;
        expanded = expanded.saturating_add(item.size());
        if expanded > MAX_TRANSFER_BYTES {
            bail!("expanded chart package exceeds 1 GiB");
        }
        let relative = item
            .enclosed_name()
            .context("archive path escapes its destination")?;
        let output = destination.join(relative);
        if item.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let canonical_parent = std::fs::canonicalize(output.parent().unwrap_or(destination))?;
        if !canonical_parent.starts_with(&canonical_destination) {
            bail!("archive entry escapes BBT Imports");
        }
        let mut file = File::create(&output)?;
        let remaining = MAX_TRANSFER_BYTES.saturating_sub(copied_total);
        let copied = std::io::copy(&mut item.take(remaining.saturating_add(1)), &mut file)?;
        if copied > remaining {
            bail!("expanded chart package exceeds 1 GiB");
        }
        copied_total = copied_total.saturating_add(copied);
    }
    Ok(())
}

fn validate_entry<R: Read>(item: &zip::read::ZipFile<'_, R>) -> Result<()> {
    let raw = item.name().replace('\\', "/");
    if raw.len() > MAX_ENTRY_NAME_BYTES {
        bail!("archive entry name exceeds the safety limit");
    }
    let path = Path::new(&raw);
    if raw.starts_with('/') || raw.starts_with("//") || raw.contains(':') {
        bail!("archive contains an absolute or device path: {raw}");
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("archive contains path traversal: {raw}");
    }
    if item
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        bail!("archive links are not allowed: {raw}");
    }
    Ok(())
}

fn executable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe"
                    | "dll"
                    | "com"
                    | "scr"
                    | "msi"
                    | "bat"
                    | "cmd"
                    | "ps1"
                    | "vbs"
                    | "js"
                    | "lua"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn archive(path: &Path, name: &str) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(b"chart").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn installs_by_hash_without_overwriting_other_charts() {
        let root = std::env::temp_dir().join(format!("bbt-transfer-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let package = root.join("chart.zip");
        archive(&package, "level.json");
        let offer = inspect_offer(&package, "Host").unwrap();
        let installed =
            install_received_package(&package, &offer.sha256, &root.join("imports"), true).unwrap();
        assert!(installed.join("level.json").is_file());
        assert_eq!(
            install_received_package(&package, &offer.sha256, &root.join("imports"), true).unwrap(),
            installed
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_path_traversal_and_requires_script_confirmation() {
        let root = std::env::temp_dir().join(format!("bbt-transfer-bad-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let bad = root.join("bad.zip");
        archive(&bad, "../escape.txt");
        assert!(inspect_offer(&bad, "Host").is_err());
        let scripted = root.join("scripted.zip");
        archive(&scripted, "events.lua");
        let offer = inspect_offer(&scripted, "Host").unwrap();
        assert!(
            install_received_package(&scripted, &offer.sha256, &root.join("imports"), false)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cache_clear_protects_the_active_chart() {
        let root = std::env::temp_dir().join(format!("bbt-cache-{}", rand::random::<u64>()));
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::create_dir_all(root.join("old")).unwrap();
        std::fs::write(root.join("active/level.json"), b"active").unwrap();
        std::fs::write(root.join("old/level.json"), b"old").unwrap();
        assert!(clear_cache(&root, Some("active")).unwrap() > 0);
        assert!(root.join("active/level.json").is_file());
        assert!(!root.join("old").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_archive_hash_mismatch_without_leaving_a_partial_install() {
        let root =
            std::env::temp_dir().join(format!("bbt-transfer-hash-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let package = root.join("chart.zip");
        archive(&package, "level.json");
        let imports = root.join("imports");
        assert!(install_received_package(&package, &"0".repeat(64), &imports, true).is_err());
        assert!(!imports.exists() || std::fs::read_dir(&imports).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn archives_a_selected_directory_for_host_fallback() {
        let root =
            std::env::temp_dir().join(format!("bbt-transfer-archive-{}", rand::random::<u64>()));
        let chart = root.join("chart");
        std::fs::create_dir_all(chart.join("audio")).unwrap();
        std::fs::write(chart.join("level.json"), b"chart").unwrap();
        std::fs::write(chart.join("audio/song.ogg"), b"audio").unwrap();
        let package = archive_chart_directory(&chart, &root.join("out/chart.zip")).unwrap();
        let offer = inspect_offer(&package, "Host").unwrap();
        assert!(offer.size > 0);
        let installed =
            install_received_package(&package, &offer.sha256, &root.join("imports"), true).unwrap();
        assert!(installed.join("level.json").is_file());
        assert!(installed.join("audio/song.ogg").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
