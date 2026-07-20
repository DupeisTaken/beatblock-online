//! Local API credential lifecycle.
//!
//! Tokens are fixed-size random values written through a same-directory
//! temporary file. A truncated credential must never become a predictable API
//! key after an interrupted write or external file corruption.

use anyhow::{Context, Result};
use rand::RngCore;
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const LOCAL_TOKEN_BYTES: usize = 32;
pub const LOCAL_TOKEN_HEX_LENGTH: usize = LOCAL_TOKEN_BYTES * 2;

pub fn is_valid_local_token(value: &str) -> bool {
    value.len() == LOCAL_TOKEN_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn load_or_create_local_token(data_dir: &Path) -> Result<String> {
    let path = data_dir.join("local-token.txt");
    // A credential file is at most 64 hexadecimal bytes plus one newline.
    // Check its size before reading so a corrupt local file cannot force an
    // unbounded startup allocation.
    let existing = std::fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.len() <= (LOCAL_TOKEN_HEX_LENGTH + 2) as u64)
        .and_then(|_| std::fs::read_to_string(&path).ok());
    if let Some(value) = existing {
        let value = value.trim();
        if is_valid_local_token(value) {
            return Ok(value.to_owned());
        }
    }
    rotate_local_token(data_dir)
}

pub fn rotate_local_token(data_dir: &Path) -> Result<String> {
    let mut bytes = [0u8; LOCAL_TOKEN_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    write_token_atomically(&data_dir.join("local-token.txt"), &token)?;
    Ok(token)
}

fn write_token_atomically(path: &Path, token: &str) -> Result<()> {
    debug_assert!(is_valid_local_token(token));
    let parent = path.parent().context("local token path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = temporary_token_path(path);
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .context("create temporary local API token")?;
        file.write_all(token.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        // Activate the durable file with the same write-through replacement
        // primitive used by exports, leaving no missing-token window.
        crate::exports::replace_file(&temporary, path).context("activate local API token")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn temporary_token_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(".local-token-{}.tmp", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bbt-credentials-{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn empty_or_corrupt_tokens_are_replaced_with_strong_credentials() {
        let root = temporary("replace");
        std::fs::create_dir_all(&root).unwrap();
        for invalid in [
            String::new(),
            "abc".to_owned(),
            "A".repeat(LOCAL_TOKEN_HEX_LENGTH),
            "x".repeat(1024),
        ] {
            std::fs::write(root.join("local-token.txt"), invalid).unwrap();
            let token = load_or_create_local_token(&root).unwrap();
            assert!(is_valid_local_token(&token));
            assert_eq!(
                std::fs::read_to_string(root.join("local-token.txt"))
                    .unwrap()
                    .trim(),
                token
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_valid_token_is_stable_until_explicit_rotation() {
        let root = temporary("rotate");
        let first = load_or_create_local_token(&root).unwrap();
        assert_eq!(load_or_create_local_token(&root).unwrap(), first);
        let second = rotate_local_token(&root).unwrap();
        assert!(is_valid_local_token(&second));
        assert_ne!(second, first);
        let _ = std::fs::remove_dir_all(root);
    }
}
