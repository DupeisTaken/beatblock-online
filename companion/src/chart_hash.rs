use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

const JUNK: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];
const MAX_CHART_FILES: usize = 20_000;
const MAX_CHART_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CHART_PATH_BYTES: usize = 1_024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartHash {
    pub algorithm: String,
    pub hash: String,
    pub package_name: String,
    pub file_count: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ManifestEntry {
    path: String,
    size: u64,
    modified_ns: u128,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheRecord {
    manifest: Vec<ManifestEntry>,
    result: ChartHash,
}

pub fn canonical_chart_hash_cached(
    path: impl AsRef<Path>,
    cache_dir: impl AsRef<Path>,
) -> Result<ChartHash> {
    let path = path.as_ref();
    let manifest = package_manifest(path)?;
    let key = hex::encode(Sha256::digest(path.to_string_lossy().as_bytes()));
    let cache_path = cache_dir.as_ref().join(format!("{key}.json"));
    if let Ok(bytes) = std::fs::read(&cache_path) {
        if let Ok(cached) = serde_json::from_slice::<CacheRecord>(&bytes) {
            if cached.manifest == manifest {
                return Ok(cached.result);
            }
        }
    }
    let result = canonical_chart_hash(path)?;
    std::fs::create_dir_all(cache_dir.as_ref())?;
    let temporary = temporary_path(&cache_path);
    let mut file = File::create(&temporary)?;
    serde_json::to_writer(
        &mut file,
        &CacheRecord {
            manifest,
            result: result.clone(),
        },
    )?;
    file.flush()?;
    if let Err(error) = crate::exports::replace_file(&temporary, &cache_path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(result)
}

pub fn canonical_chart_hash(path: impl AsRef<Path>) -> Result<ChartHash> {
    let path = path.as_ref();
    let mut hasher = Sha256::new();
    hasher.update(b"beatblock-online-chart-package-v1\0");
    let file_count = if path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        hash_zip_entries(path, &mut hasher)?
    } else {
        hash_directory_entries(path, &mut hasher)?
    };
    Ok(ChartHash {
        algorithm: "sha256-canonical-package-v1".into(),
        hash: hex::encode(hasher.finalize()),
        package_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        file_count,
    })
}

fn hash_directory_entries(path: &Path, hasher: &mut Sha256) -> Result<usize> {
    let mut entries = Vec::new();
    let mut total_size = 0u64;
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry.with_context(|| format!("walk chart package {}", path.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if is_junk(entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(path)?
            .to_string_lossy()
            .replace('\\', "/");
        if relative.len() > MAX_CHART_PATH_BYTES {
            anyhow::bail!("chart entry path exceeds the safety limit");
        }
        let size = entry.metadata()?.len();
        total_size = total_size.saturating_add(size);
        if entries.len() >= MAX_CHART_FILES || total_size > MAX_CHART_BYTES {
            anyhow::bail!("chart package exceeds the 20,000-file or 1 GiB safety limit");
        }
        entries.push((relative, entry.path().to_owned()));
    }
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for (name, path) in &entries {
        let mut file = File::open(path).with_context(|| format!("read {}", path.display()))?;
        let size = file.metadata()?.len();
        hash_entry(name, size, &mut file, hasher)?;
    }
    Ok(entries.len())
}

fn hash_zip_entries(path: &Path, hasher: &mut Sha256) -> Result<usize> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() > MAX_CHART_FILES {
        anyhow::bail!("chart package exceeds the 20,000-file safety limit");
    }
    let mut entries = Vec::new();
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let item = archive.by_index(index)?;
        if item.is_dir() {
            continue;
        }
        let name = item.name().replace('\\', "/");
        if name.len() > MAX_CHART_PATH_BYTES {
            anyhow::bail!("chart entry path exceeds the safety limit");
        }
        if is_junk(Path::new(&name)) {
            continue;
        }
        total_size = total_size.saturating_add(item.size());
        if entries.len() >= MAX_CHART_FILES || total_size > MAX_CHART_BYTES {
            anyhow::bail!("chart package exceeds the 20,000-file or 1 GiB safety limit");
        }
        entries.push((name, index, item.size()));
    }
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for (name, index, size) in &entries {
        let mut item = archive.by_index(*index)?;
        hash_entry(name, *size, &mut item, hasher)?;
    }
    Ok(entries.len())
}

fn hash_entry(
    name: &str,
    expected_size: u64,
    reader: &mut impl Read,
    hasher: &mut Sha256,
) -> Result<()> {
    hasher.update((name.len() as u64).to_le_bytes());
    hasher.update(name.as_bytes());
    hasher.update(expected_size.to_le_bytes());
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    if total != expected_size {
        anyhow::bail!("chart entry changed while it was being hashed");
    }
    Ok(())
}

fn is_junk(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|value| JUNK.iter().any(|junk| value.eq_ignore_ascii_case(junk)))
}

fn package_manifest(path: &Path) -> Result<Vec<ManifestEntry>> {
    let mut result = Vec::new();
    if path.is_file() {
        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_CHART_BYTES {
            anyhow::bail!("chart package exceeds the 1 GiB safety limit");
        }
        result.push(ManifestEntry {
            path: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size: metadata.len(),
            modified_ns: modified_ns(&metadata),
        });
    } else {
        let mut total_size = 0u64;
        for entry in WalkDir::new(path).follow_links(false) {
            let entry = entry.with_context(|| format!("walk chart package {}", path.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if is_junk(entry.path()) {
                continue;
            }
            let metadata = entry.metadata()?;
            let relative = entry
                .path()
                .strip_prefix(path)?
                .to_string_lossy()
                .replace('\\', "/");
            if relative.len() > MAX_CHART_PATH_BYTES {
                anyhow::bail!("chart entry path exceeds the safety limit");
            }
            total_size = total_size.saturating_add(metadata.len());
            if result.len() >= MAX_CHART_FILES || total_size > MAX_CHART_BYTES {
                anyhow::bail!("chart package exceeds the 20,000-file or 1 GiB safety limit");
            }
            result.push(ManifestEntry {
                path: relative,
                size: metadata.len(),
                modified_ns: modified_ns(&metadata),
            });
        }
    }
    result.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    Ok(result)
}

fn modified_ns(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignores_os_junk_and_path_order() {
        let root = std::env::temp_dir().join(format!("bbt-hash-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("b.json"), b"two").unwrap();
        std::fs::write(root.join("a.json"), b"one").unwrap();
        let first = canonical_chart_hash(&root).unwrap();
        std::fs::write(root.join("Thumbs.db"), b"junk").unwrap();
        let second = canonical_chart_hash(&root).unwrap();
        assert_eq!(first.hash, second.hash);
        let _ = std::fs::remove_dir_all(root);
    }
}
