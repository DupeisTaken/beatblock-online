use anyhow::{Context, Result};
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const RELEASES_URL: &str =
    "https://api.github.com/repos/DupeisTaken/beatblock-online/releases?per_page=20";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current_version: Version,
    pub latest_version: Option<Version>,
    pub release_url: Option<String>,
}

impl UpdateCheck {
    pub fn update_available(&self) -> bool {
        self.latest_version
            .as_ref()
            .is_some_and(|latest| latest > &self.current_version)
    }

    pub fn status(&self) -> String {
        match self.latest_version.as_ref() {
            Some(latest) if self.update_available() => {
                format!("Version {latest} is available. Open the release page to update.")
            }
            Some(latest) => format!("You are up to date. Latest compatible release: {latest}."),
            None if self.current_version.pre.is_empty() => {
                "No published stable release was found.".into()
            }
            None => "No published release was found for this preview channel.".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
}

pub fn check_for_updates() -> Result<UpdateCheck> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build update client")?;
    let response = client
        .get(RELEASES_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(
            reqwest::header::USER_AGENT,
            concat!("BeatblockOnlineInstaller/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .context("contact GitHub Releases")?
        .error_for_status()
        .context("GitHub Releases returned an error")?;
    evaluate_releases(env!("CARGO_PKG_VERSION"), &response.text()?)
}

fn evaluate_releases(current: &str, body: &str) -> Result<UpdateCheck> {
    let current_version = Version::parse(current).context("parse installed version")?;
    let releases: Vec<Release> =
        serde_json::from_str(body).context("read the GitHub Releases response")?;
    let include_prereleases = !current_version.pre.is_empty();
    let latest = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let tag = release
                .tag_name
                .trim_start_matches(|character| character == 'v' || character == 'V');
            let version = Version::parse(tag).ok()?;
            if (release.prerelease || !version.pre.is_empty()) && !include_prereleases {
                return None;
            }
            Some((version, release.html_url))
        })
        .max_by(|left, right| left.0.cmp(&right.0));

    Ok(UpdateCheck {
        current_version,
        latest_version: latest.as_ref().map(|(version, _)| version.clone()),
        release_url: latest.map(|(_, url)| url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_build_finds_a_newer_preview_release() {
        let check = evaluate_releases(
            "0.3.0-beta.3",
            r#"[
                {"tag_name":"v0.3.0-beta.4","html_url":"https://example.test/beta-4","draft":false,"prerelease":true},
                {"tag_name":"not-a-version","html_url":"https://example.test/notes","draft":false,"prerelease":false},
                {"tag_name":"v0.4.0-beta.1","html_url":"https://example.test/draft","draft":true,"prerelease":true}
            ]"#,
        )
        .unwrap();

        assert!(check.update_available());
        assert_eq!(
            check.latest_version,
            Some(Version::parse("0.3.0-beta.4").unwrap())
        );
        assert_eq!(
            check.release_url.as_deref(),
            Some("https://example.test/beta-4")
        );
    }

    #[test]
    fn stable_build_ignores_prereleases_and_reports_current_release() {
        let check = evaluate_releases(
            "1.0.0",
            r#"[
                {"tag_name":"v1.1.0-beta.1","html_url":"https://example.test/beta","draft":false,"prerelease":true},
                {"tag_name":"v1.0.0","html_url":"https://example.test/stable","draft":false,"prerelease":false}
            ]"#,
        )
        .unwrap();

        assert!(!check.update_available());
        assert_eq!(check.latest_version, Some(Version::new(1, 0, 0)));
        assert!(check.status().contains("up to date"));
    }

    #[test]
    fn release_comparison_uses_semver_instead_of_lexical_tag_order() {
        let check = evaluate_releases(
            "0.9.0",
            r#"[
                {"tag_name":"v0.10.0","html_url":"https://example.test/new","draft":false,"prerelease":false},
                {"tag_name":"v0.9.9","html_url":"https://example.test/old","draft":false,"prerelease":false}
            ]"#,
        )
        .unwrap();

        assert_eq!(check.latest_version, Some(Version::new(0, 10, 0)));
        assert!(check.update_available());
    }
}
