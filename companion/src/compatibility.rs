use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

/// The newest Beatblock version exercised by this Beatblock Online release.
///
/// This is release-note metadata, not an installer allowlist. Newer Beatblock
/// builds remain installable by default and identify themselves at runtime.
pub const TESTED_BEATBLOCK_VERSION: &str = "1.7.1a";
pub const TESTED_BEATBLOCK_BUILD_ID: &str = "d40b7083";
pub const COMPATIBILITY_ISSUE_URL: &str =
    "https://github.com/DupeisTaken/beatblock-online/issues/new?template=beatblock_compatibility.yml";

const MAX_DISPLAYED_VERSION_CHARS: usize = 160;
const GAME_CONTENT_FILES: &[&str] = &[
    "Beatblock.exe",
    "project.lua",
    "packed/data.zip",
    "packed/levelformat.zip",
    "packed/lib.zip",
    "packed/obj.zip",
    "packed/preload.zip",
    "packed/states.zip",
    "packed/threads.zip",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameBuildIdentitySource {
    DisplayedBuildHash,
    DisplayedVersionDigest,
    GameContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBuildIdentity {
    pub displayed_version: String,
    pub build_id: String,
    pub source: GameBuildIdentitySource,
}

impl GameBuildIdentity {
    /// Turns the exact version Beatblock draws in its top-right corner into a
    /// stable room identity. Current upstream builds end in a bracketed short
    /// Git hash, for example `1.7.1a (Early Access)[d40b7083]`.
    pub fn from_displayed_version(displayed: &str) -> Result<Self> {
        let displayed = displayed.trim();
        if displayed.is_empty() {
            bail!("Beatblock did not expose its displayed version");
        }
        if displayed.chars().count() > MAX_DISPLAYED_VERSION_CHARS
            || displayed.chars().any(char::is_control)
        {
            bail!("Beatblock displayed version is invalid");
        }
        if let Some(build_id) = bracketed_build_id(displayed) {
            return Ok(Self {
                displayed_version: displayed.to_owned(),
                build_id,
                source: GameBuildIdentitySource::DisplayedBuildHash,
            });
        }

        // A future upstream version may change the label format. Hashing the
        // complete displayed value preserves exact same-build matching without
        // requiring a Beatblock Online release for each upstream build.
        Ok(Self {
            displayed_version: displayed.to_owned(),
            build_id: format!(
                "version-{}",
                hex::encode(Sha256::digest(displayed.as_bytes()))
            ),
            source: GameBuildIdentitySource::DisplayedVersionDigest,
        })
    }

    /// Last-resort identity when a future Beatblock build removes the displayed
    /// version global. The digest covers code-bearing game files, never user
    /// charts or saves, and therefore needs no per-release registry.
    pub fn from_game_directory(game_directory: &Path) -> Result<Self> {
        let mut digest = Sha256::new();
        let mut included = 0usize;
        for relative in GAME_CONTENT_FILES {
            let path = game_directory.join(relative);
            if !path.is_file() {
                continue;
            }
            let mut file = File::open(&path)
                .with_context(|| format!("open Beatblock build input {}", path.display()))?;
            let length = file
                .metadata()
                .with_context(|| format!("inspect Beatblock build input {}", path.display()))?
                .len();
            digest.update((relative.len() as u64).to_le_bytes());
            digest.update(relative.as_bytes());
            digest.update(length.to_le_bytes());
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .with_context(|| format!("read Beatblock build input {}", path.display()))?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            included += 1;
        }
        if included == 0 {
            bail!("Beatblock build identity is unavailable");
        }
        let build_id = format!("content-{}", hex::encode(digest.finalize()));
        Ok(Self {
            displayed_version: "Unknown Beatblock version".into(),
            build_id,
            source: GameBuildIdentitySource::GameContentDigest,
        })
    }

    pub fn short_build_id(&self) -> &str {
        self.build_id
            .strip_prefix("version-")
            .or_else(|| self.build_id.strip_prefix("content-"))
            .unwrap_or(&self.build_id)
            .get(..8)
            .unwrap_or(&self.build_id)
    }

    pub fn validate(&self) -> Result<()> {
        let displayed = self.displayed_version.trim();
        if displayed.is_empty()
            || displayed.chars().count() > MAX_DISPLAYED_VERSION_CHARS
            || displayed.chars().any(char::is_control)
        {
            bail!("Beatblock displayed version is invalid");
        }
        if !(7..=80).contains(&self.build_id.len())
            || !self
                .build_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("Beatblock build id is invalid");
        }
        Ok(())
    }
}

fn bracketed_build_id(displayed: &str) -> Option<String> {
    let opening = displayed.rfind('[')?;
    if !displayed.ends_with(']') {
        return None;
    }
    let candidate = &displayed[opening + 1..displayed.len() - 1];
    (7..=40)
        .contains(&candidate.len())
        .then_some(candidate)
        .filter(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

pub fn tested_beatblock_label() -> String {
    format!("{TESTED_BEATBLOCK_VERSION}+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_upstream_hash_is_the_primary_build_identity() {
        let identity =
            GameBuildIdentity::from_displayed_version("1.7.1a (Early Access)[D40B7083]").unwrap();
        assert_eq!(
            identity.displayed_version,
            "1.7.1a (Early Access)[D40B7083]"
        );
        assert_eq!(identity.build_id, "d40b7083");
        assert_eq!(identity.source, GameBuildIdentitySource::DisplayedBuildHash);
        assert_eq!(identity.short_build_id(), "d40b7083");
        identity.validate().unwrap();
    }

    #[test]
    fn changed_display_format_still_gets_a_maintenance_free_identity() {
        let first =
            GameBuildIdentity::from_displayed_version("Beatblock nightly 2026-08-01").unwrap();
        let second =
            GameBuildIdentity::from_displayed_version("Beatblock nightly 2026-08-02").unwrap();
        assert_eq!(
            first.source,
            GameBuildIdentitySource::DisplayedVersionDigest
        );
        assert_ne!(first.build_id, second.build_id);
        assert_eq!(first.build_id.len(), "version-".len() + 64);
        first.validate().unwrap();
    }

    #[test]
    fn malformed_or_empty_display_values_are_rejected() {
        assert!(GameBuildIdentity::from_displayed_version("").is_err());
        assert!(GameBuildIdentity::from_displayed_version("1.7.1a[d40b7083]\nforged").is_err());
        assert_eq!(
            GameBuildIdentity::from_displayed_version("1.7.1a[not-a-hash]")
                .unwrap()
                .source,
            GameBuildIdentitySource::DisplayedVersionDigest
        );
        assert!(GameBuildIdentity {
            displayed_version: "forged\nversion".into(),
            build_id: "d40b7083".into(),
            source: GameBuildIdentitySource::DisplayedBuildHash,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn content_fallback_changes_with_game_code() {
        let root = std::env::temp_dir().join(format!(
            "bbt-game-content-identity-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(root.join("packed")).unwrap();
        std::fs::write(root.join("Beatblock.exe"), b"launcher").unwrap();
        std::fs::write(root.join("packed/states.zip"), b"states-a").unwrap();
        let first = GameBuildIdentity::from_game_directory(&root).unwrap();
        std::fs::write(root.join("packed/states.zip"), b"states-b").unwrap();
        let second = GameBuildIdentity::from_game_directory(&root).unwrap();
        assert_eq!(first.source, GameBuildIdentitySource::GameContentDigest);
        assert_ne!(first.build_id, second.build_id);
        let _ = std::fs::remove_dir_all(root);
    }
}
