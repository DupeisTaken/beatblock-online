use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const MAX_TRANSFER_BYTES: u64 = 1024 * 1024 * 1024;
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
    Ok(destination)
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
}
