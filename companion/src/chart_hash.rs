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
    modified_ms: u128,
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
    crate::exports::replace_file(&temporary, &cache_path)?;
    Ok(result)
}

pub fn canonical_chart_hash(path: impl AsRef<Path>) -> Result<ChartHash> {
    let path = path.as_ref();
    let mut entries = if path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        zip_entries(path)?
    } else {
        directory_entries(path)?
    };
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut hasher = Sha256::new();
    hasher.update(b"beatblock-together-chart-package-v1\0");
    for (name, bytes) in &entries {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    Ok(ChartHash {
        algorithm: "sha256-canonical-package-v1".into(),
        hash: hex::encode(hasher.finalize()),
        package_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        file_count: entries.len(),
    })
}

fn directory_entries(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut result = Vec::new();
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        if is_junk(entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(path)?
            .to_string_lossy()
            .replace('\\', "/");
        result.push((
            relative,
            std::fs::read(entry.path())
                .with_context(|| format!("read {}", entry.path().display()))?,
        ));
    }
    Ok(result)
}

fn zip_entries(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut result = Vec::new();
    for index in 0..archive.len() {
        let mut item = archive.by_index(index)?;
        if item.is_dir() {
            continue;
        }
        let name = item.name().replace('\\', "/");
        if is_junk(Path::new(&name)) {
            continue;
        }
        let mut bytes = Vec::with_capacity(item.size() as usize);
        item.read_to_end(&mut bytes)?;
        result.push((name, bytes));
    }
    Ok(result)
}

fn is_junk(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|value| JUNK.iter().any(|junk| value.eq_ignore_ascii_case(junk)))
}

fn package_manifest(path: &Path) -> Result<Vec<ManifestEntry>> {
    let mut result = Vec::new();
    if path.is_file() {
        let metadata = std::fs::metadata(path)?;
        result.push(ManifestEntry {
            path: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        });
    } else {
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            if is_junk(entry.path()) {
                continue;
            }
            let metadata = entry.metadata()?;
            result.push(ManifestEntry {
                path: entry
                    .path()
                    .strip_prefix(path)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                size: metadata.len(),
                modified_ms: modified_ms(&metadata),
            });
        }
    }
    result.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    Ok(result)
}

fn modified_ms(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_millis())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".tmp");
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
