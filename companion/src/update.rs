use anyhow::{bail, Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
use url::Url;
use uuid::Uuid;

const RELEASES_URL: &str =
    "https://api.github.com/repos/DupeisTaken/beatblock-online/releases?per_page=20";
pub const RELEASES_PAGE: &str = "https://github.com/DupeisTaken/beatblock-online/releases";
const INSTALLER_ASSET: &str = "BeatblockOnlineInstaller.exe";
const CHECKSUM_ASSET: &str = "SHA256SUMS.txt";
const MAX_INSTALLER_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 64 * 1024;
const MAX_RECEIPT_BYTES: u64 = 16 * 1024;
const MAX_RELEASE_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const UPDATE_READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRelease {
    pub version: Version,
    pub release_url: String,
    pub installer: ReleaseAsset,
    pub checksums: ReleaseAsset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub current_version: Version,
    pub latest_version: Option<Version>,
    pub release_url: Option<String>,
    pub release: Option<UpdateRelease>,
}

impl UpdateCheck {
    pub fn update_available(&self) -> bool {
        self.latest_version
            .as_ref()
            .is_some_and(|latest| latest > &self.current_version)
    }

    pub fn installable_update(&self) -> Option<&UpdateRelease> {
        self.update_available().then_some(())?;
        self.release.as_ref()
    }

    pub fn status(&self) -> String {
        match self.latest_version.as_ref() {
            Some(latest) if self.update_available() && self.release.is_some() => format!(
                "Version {latest} is available. Its installer and checksums are ready to verify."
            ),
            Some(latest) if self.update_available() => format!(
                "Version {latest} is available, but its installer assets are incomplete. View the release notes for manual instructions."
            ),
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
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedUpdate {
    pub version: Version,
    pub receipt_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpdateReceipt {
    update_id: Uuid,
    expected_version: String,
    expected_sha256: String,
    staged_path: PathBuf,
    managed_destination: PathBuf,
    parent_pid: u32,
}

struct StagingCleanup {
    staged_dir: PathBuf,
    receipt_path: PathBuf,
    armed: bool,
}

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = fs::remove_file(self.staged_dir.join(INSTALLER_ASSET));
        let _ = fs::remove_file(&self.receipt_path);
        let _ = fs::remove_dir(&self.staged_dir);
    }
}

pub fn check_for_updates() -> Result<UpdateCheck> {
    let client = update_client()?;
    let mut response = client
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
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELEASE_METADATA_BYTES)
    {
        bail!("GitHub Releases response exceeds the metadata size limit");
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(MAX_RELEASE_METADATA_BYTES + 1)
        .read_to_end(&mut body)
        .context("read GitHub Releases response")?;
    if body.len() as u64 > MAX_RELEASE_METADATA_BYTES {
        bail!("GitHub Releases response exceeds the metadata size limit");
    }
    evaluate_releases(
        env!("CARGO_PKG_VERSION"),
        std::str::from_utf8(&body).context("GitHub Releases response is not UTF-8")?,
    )
}

pub fn prepare_update(data_dir: &Path) -> Result<PreparedUpdate> {
    ensure_self_update_allowed()?;
    let check = check_for_updates()?;
    let release = check
        .installable_update()
        .context("no newer release with a complete installer asset set is available")?;
    stage_release(data_dir, release, std::process::id())
}

pub fn launch_finalizer(prepared: &PreparedUpdate, data_dir: &Path) -> Result<()> {
    ensure_self_update_allowed()?;
    let receipt = read_and_validate_receipt(data_dir, &prepared.receipt_path, false)?;
    validate_staged_binary(
        &receipt.staged_path,
        &receipt.expected_version,
        &receipt.expected_sha256,
        false,
    )?;
    let launch = Command::new(&receipt.staged_path)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--finalize-update")
        .arg(&prepared.receipt_path)
        .spawn()
        .context("start the verified installer update finalizer");
    if let Err(error) = launch {
        // These exact paths have just passed the managed receipt contract and
        // the staged process never started, so retaining them cannot help a
        // retry and only creates an orphan.
        let _ = fs::remove_file(&receipt.staged_path);
        let _ = fs::remove_file(&prepared.receipt_path);
        if let Some(parent) = receipt.staged_path.parent() {
            let _ = fs::remove_dir(parent);
        }
        return Err(error);
    }
    Ok(())
}

pub fn finalize_update(data_dir: &Path, receipt_path: &Path) -> Result<PathBuf> {
    ensure_self_update_allowed()?;
    let receipt = read_and_validate_receipt(data_dir, receipt_path, true)?;
    wait_for_process_exit(receipt.parent_pid)?;
    // Revalidate every parent after waiting for the old process. The receipt
    // cannot be used to swap a symlink into place during that window.
    validate_receipt_paths(data_dir, receipt_path, &receipt, true)?;
    validate_staged_binary(
        &receipt.staged_path,
        &receipt.expected_version,
        &receipt.expected_sha256,
        false,
    )?;
    if fs::symlink_metadata(ready_path(data_dir, receipt.update_id)).is_ok() {
        bail!("update readiness marker already exists");
    }
    let ready_token = hex::encode(rand::random::<[u8; 32]>());
    promote_and_launch(&receipt, |path| {
        let mut child = Command::new(path)
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--cleanup-update")
            .arg(receipt.update_id.to_string())
            .arg("--cleanup-parent")
            .arg(std::process::id().to_string())
            .arg("--update-ready-token")
            .arg(&ready_token)
            .spawn()
            .context("restart the managed installer")?;
        let ready = wait_for_ready_ack(data_dir, &receipt, &ready_token, &mut child);
        if ready.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        ready
    })?;
    Ok(receipt.managed_destination)
}

pub fn ensure_self_update_allowed() -> Result<()> {
    reject_elevated_self_update(process_is_elevated()?)
}

fn reject_elevated_self_update(elevated: bool) -> Result<()> {
    if elevated {
        bail!(
            "Installer self-update is disabled while running as administrator. Close this window, reopen BeatblockOnlineInstaller.exe normally, and choose Update Installer again"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn process_is_elevated() -> Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        bail!("could not inspect installer elevation state");
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(token) };
    if result == 0 || returned != std::mem::size_of::<TOKEN_ELEVATION>() as u32 {
        bail!("could not read installer elevation state");
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(not(windows))]
fn process_is_elevated() -> Result<bool> {
    Ok(false)
}

pub fn acknowledge_update_ready(data_dir: &Path, update_id: Uuid, ready_token: &str) -> Result<()> {
    validate_sha256(ready_token).context("update readiness token is invalid")?;
    let receipt_path = data_dir
        .join("updates/receipts")
        .join(format!("{update_id}.json"));
    let receipt = read_and_validate_receipt(data_dir, &receipt_path, false)?;
    if receipt.update_id != update_id {
        bail!("update readiness ID does not match its receipt");
    }
    if fs::canonicalize(std::env::current_exe()?)?
        != fs::canonicalize(&receipt.managed_destination)?
    {
        bail!("only the promoted managed installer may acknowledge readiness");
    }
    validate_staged_binary(
        &receipt.managed_destination,
        &receipt.expected_version,
        &receipt.expected_sha256,
        false,
    )?;
    let ready_path = ready_path(data_dir, update_id);
    let temporary = ready_path.with_extension("ready.tmp");
    write_new_file(&temporary, ready_token.as_bytes())?;
    fs::rename(&temporary, &ready_path).context("publish managed installer readiness")
}

pub fn cleanup_completed_update(data_dir: &Path, update_id: Uuid, parent_pid: u32) -> Result<()> {
    if parent_pid == 0 || parent_pid == std::process::id() {
        bail!("cleanup parent process is invalid");
    }
    wait_for_process_exit(parent_pid)?;
    let updates_root = data_dir.join("updates");
    let receipts_root = updates_root.join("receipts");
    let staged_root = updates_root.join("staged");
    let installer_root = data_dir.join("installer");
    for directory in [&updates_root, &receipts_root, &staged_root, &installer_root] {
        let metadata = fs::symlink_metadata(directory)
            .with_context(|| format!("inspect {}", directory.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("managed cleanup directory is not a real directory");
        }
    }
    let stage_dir = staged_root.join(update_id.to_string());
    if let Ok(stage_metadata) = fs::symlink_metadata(&stage_dir) {
        if !stage_metadata.is_dir() || stage_metadata.file_type().is_symlink() {
            bail!("update staging directory cannot be a symlink");
        }
        let staged = stage_dir.join(INSTALLER_ASSET);
        if staged.is_file() {
            fs::remove_file(&staged).context("remove completed staged installer")?;
        }
        fs::remove_dir(&stage_dir).context("remove completed update staging directory")?;
    }
    let receipt = receipts_root.join(format!("{update_id}.json"));
    if receipt.is_file() {
        fs::remove_file(receipt).context("remove completed update receipt")?;
    }
    let ready = ready_path(data_dir, update_id);
    if ready.is_file() {
        fs::remove_file(ready).context("remove completed update readiness marker")?;
    }
    let ready_temporary = ready_path(data_dir, update_id).with_extension("ready.tmp");
    if ready_temporary.is_file() {
        fs::remove_file(ready_temporary).context("remove incomplete update readiness marker")?;
    }
    let temporary = installer_root.join(format!("BeatblockOnlineInstaller.{update_id}.tmp"));
    if temporary.is_file() {
        fs::remove_file(temporary).context("remove completed update temporary file")?;
    }
    let backup = installer_root.join(format!("BeatblockOnlineInstaller.{update_id}.previous.exe"));
    if backup.is_file() {
        fs::remove_file(backup).context("remove prior installer after successful relaunch")?;
    }
    Ok(())
}

fn ready_path(data_dir: &Path, update_id: Uuid) -> PathBuf {
    data_dir
        .join("updates/receipts")
        .join(format!("{update_id}.ready"))
}

fn wait_for_ready_ack(
    data_dir: &Path,
    receipt: &UpdateReceipt,
    expected_token: &str,
    child: &mut std::process::Child,
) -> Result<()> {
    let path = ready_path(data_dir, receipt.update_id);
    let deadline = Instant::now() + UPDATE_READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("updated installer exited before becoming ready ({status})");
        }
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 64 {
                let _ = child.kill();
                let _ = child.wait();
                bail!("updated installer published an invalid readiness marker");
            }
            let value = fs::read_to_string(&path)?;
            if value != expected_token {
                let _ = child.kill();
                let _ = child.wait();
                bail!("updated installer readiness marker has the wrong hash");
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "updated installer did not become ready within {} seconds",
                UPDATE_READY_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn update_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // The current installer asset is large enough that slower connections
        // need more than the short metadata-check window.
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("too many release-asset redirects");
            }
            if trusted_download_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("release-asset redirect left trusted HTTPS hosting")
            }
        }))
        .build()
        .context("build update client")
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
            // Current Git refs are raw SemVer. Accept one historical `v`
            // prefix so cached metadata and older mirrors remain readable.
            let tag = release
                .tag_name
                .strip_prefix('v')
                .or_else(|| release.tag_name.strip_prefix('V'))
                .unwrap_or(&release.tag_name);
            let version = Version::parse(tag).ok()?;
            if (release.prerelease || !version.pre.is_empty()) && !include_prereleases {
                return None;
            }
            Some((version, release))
        })
        .max_by(|left, right| left.0.cmp(&right.0));

    let release_url = latest.as_ref().map(|(_, release)| release.html_url.clone());
    let latest_version = latest.as_ref().map(|(version, _)| version.clone());
    let release = latest
        .and_then(|(version, release)| complete_release(version, release).transpose())
        .transpose()?;

    Ok(UpdateCheck {
        current_version,
        latest_version,
        release_url,
        release,
    })
}

fn complete_release(version: Version, release: Release) -> Result<Option<UpdateRelease>> {
    let installers = release
        .assets
        .iter()
        .filter(|asset| asset.name == INSTALLER_ASSET)
        .collect::<Vec<_>>();
    let checksums = release
        .assets
        .iter()
        .filter(|asset| asset.name == CHECKSUM_ASSET)
        .collect::<Vec<_>>();
    if installers.is_empty() || checksums.is_empty() {
        return Ok(None);
    }
    if installers.len() != 1 || checksums.len() != 1 {
        bail!("release {version} contains duplicate required update assets");
    }
    let installer = release_asset(installers[0], MAX_INSTALLER_BYTES, true)?;
    let checksums = release_asset(checksums[0], MAX_CHECKSUM_BYTES, false)?;
    Ok(Some(UpdateRelease {
        version,
        release_url: release.html_url,
        installer,
        checksums,
    }))
}

fn release_asset(asset: &Asset, cap: u64, require_digest: bool) -> Result<ReleaseAsset> {
    validate_download_url(&asset.browser_download_url)?;
    if asset.size == 0 || asset.size > cap {
        bail!(
            "{} declares an invalid size of {} bytes",
            asset.name,
            asset.size
        );
    }
    let sha256 = match asset.digest.as_deref() {
        Some(digest) => Some(normalize_github_digest(digest)?),
        None if require_digest => bail!("{} has no GitHub SHA-256 digest", asset.name),
        None => None,
    };
    Ok(ReleaseAsset {
        name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
        sha256,
    })
}

fn normalize_github_digest(digest: &str) -> Result<String> {
    let value = digest
        .strip_prefix("sha256:")
        .context("GitHub asset digest is not SHA-256")?
        .to_ascii_lowercase();
    validate_sha256(&value)?;
    Ok(value)
}

fn validate_download_url(value: &str) -> Result<()> {
    let url = Url::parse(value).context("release asset has an invalid download URL")?;
    if !trusted_download_url(&url) {
        bail!("release asset must stay on trusted GitHub HTTPS hosting");
    }
    Ok(())
}

fn trusted_download_url(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    host == "github.com" || host.ends_with(".githubusercontent.com")
}

fn stage_release(
    data_dir: &Path,
    release: &UpdateRelease,
    parent_pid: u32,
) -> Result<PreparedUpdate> {
    let client = update_client()?;
    let update_id = Uuid::new_v4();
    let updates_root = data_dir.join("updates");
    let staged_root = updates_root.join("staged");
    let receipts_root = updates_root.join("receipts");
    let staged_dir = staged_root.join(update_id.to_string());
    let receipt_path = receipts_root.join(format!("{update_id}.json"));
    create_managed_directory(&updates_root)?;
    create_managed_directory(&staged_root)?;
    create_managed_directory(&receipts_root)?;
    fs::create_dir(&staged_dir).context("create isolated installer update staging directory")?;
    let mut cleanup = StagingCleanup {
        staged_dir: staged_dir.clone(),
        receipt_path: receipt_path.clone(),
        armed: true,
    };

    let installer_bytes = download_asset(&client, &release.installer, MAX_INSTALLER_BYTES)
        .context("download installer update")?;
    let checksums = download_asset(&client, &release.checksums, MAX_CHECKSUM_BYTES)
        .context("download release checksums")?;
    let github_digest = release
        .installer
        .sha256
        .as_deref()
        .context("installer asset has no GitHub SHA-256 digest")?;
    let computed = verify_installer_hashes(&installer_bytes, github_digest, &checksums)?;

    let staged_path = staged_dir.join(INSTALLER_ASSET);
    write_new_file(&staged_path, &installer_bytes)?;
    validate_staged_binary(&staged_path, &release.version.to_string(), &computed, true)?;
    let managed_destination = data_dir.join("installer").join(INSTALLER_ASSET);
    let receipt = UpdateReceipt {
        update_id,
        expected_version: release.version.to_string(),
        expected_sha256: computed,
        staged_path,
        managed_destination,
        parent_pid,
    };
    let receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    write_new_file(&receipt_path, &receipt_bytes)?;
    // Validate the complete on-disk handoff before executing it.
    read_and_validate_receipt(data_dir, &receipt_path, false)?;
    let prepared = PreparedUpdate {
        version: release.version.clone(),
        receipt_path,
    };
    cleanup.armed = false;
    Ok(prepared)
}

fn verify_installer_hashes(
    installer: &[u8],
    github_digest: &str,
    checksums: &[u8],
) -> Result<String> {
    validate_sha256(github_digest)?;
    let computed = hex::encode(Sha256::digest(installer));
    if computed != github_digest {
        bail!("downloaded installer does not match GitHub's asset digest");
    }
    let checksum_digest = checksum_for_installer(checksums)?;
    if computed != checksum_digest {
        bail!("downloaded installer does not match SHA256SUMS.txt");
    }
    Ok(computed)
}

fn download_asset(
    client: &reqwest::blocking::Client,
    asset: &ReleaseAsset,
    cap: u64,
) -> Result<Vec<u8>> {
    validate_download_url(&asset.download_url)?;
    if asset.size == 0 || asset.size > cap {
        bail!("{} exceeds the download size limit", asset.name);
    }
    let mut response = client
        .get(&asset.download_url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("BeatblockOnlineInstaller/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .with_context(|| format!("download {}", asset.name))?
        .error_for_status()
        .with_context(|| format!("download {} returned an error", asset.name))?;
    if !trusted_download_url(response.url()) {
        bail!(
            "{} download ended outside trusted HTTPS hosting",
            asset.name
        );
    }
    if response.content_length().is_some_and(|length| length > cap) {
        bail!("{} response exceeds the download size limit", asset.name);
    }
    let mut bytes = Vec::with_capacity(asset.size.min(cap) as usize);
    response
        .by_ref()
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {}", asset.name))?;
    if bytes.len() as u64 > cap {
        bail!("{} response exceeds the download size limit", asset.name);
    }
    if bytes.len() as u64 != asset.size {
        bail!("{} size differs from GitHub release metadata", asset.name);
    }
    Ok(bytes)
}

fn checksum_for_installer(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("SHA256SUMS.txt is not UTF-8")?;
    let mut matches = Vec::new();
    for line in text.lines() {
        let Some((digest, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim_start().trim_start_matches('*');
        if name == INSTALLER_ASSET {
            let digest = digest.to_ascii_lowercase();
            validate_sha256(&digest)?;
            matches.push(digest);
        }
    }
    match matches.as_slice() {
        [digest] => Ok(digest.clone()),
        [] => bail!("SHA256SUMS.txt has no exact {INSTALLER_ASSET} entry"),
        _ => bail!("SHA256SUMS.txt has duplicate {INSTALLER_ASSET} entries"),
    }
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 value");
    }
    Ok(())
}

fn validate_staged_binary(
    path: &Path,
    expected_version: &str,
    expected_sha256: &str,
    check_reported_version: bool,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("inspect staged installer")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("staged installer must be a regular file");
    }
    if metadata.len() == 0 || metadata.len() > MAX_INSTALLER_BYTES {
        bail!("staged installer has an invalid size");
    }
    let bytes = fs::read(path).context("read staged installer")?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected_sha256 {
        bail!("staged installer hash changed after download");
    }
    validate_pe_x64(&bytes)?;
    if check_reported_version {
        verify_reported_version(path, expected_version)?;
    }
    Ok(())
}

fn validate_pe_x64(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 0x40 || &bytes[..2] != b"MZ" {
        bail!("installer asset is not a Windows PE executable");
    }
    let pe_offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let header_end = pe_offset
        .checked_add(6)
        .context("PE header offset overflow")?;
    if header_end > bytes.len() || &bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
        bail!("installer asset has an invalid PE header");
    }
    let machine = u16::from_le_bytes(bytes[pe_offset + 4..pe_offset + 6].try_into().unwrap());
    if machine != 0x8664 {
        bail!("installer asset is not an x64 executable");
    }
    Ok(())
}

fn verify_reported_version(path: &Path, expected_version: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let mut command = Command::new(path);
        command.arg("--version");
        let output = command_output_with_timeout(command, Duration::from_secs(5))
            .context("query staged installer version")?;
        if !output.status.success() {
            bail!("staged installer did not report its version");
        }
        let reported = String::from_utf8_lossy(&output.stdout);
        if !reported
            .split_whitespace()
            .any(|part| part == expected_version)
        {
            bail!("staged installer version does not match the selected release");
        }
    }
    #[cfg(not(windows))]
    let _ = (path, expected_version);
    Ok(())
}

fn command_output_with_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start version probe")?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child
                .wait_with_output()
                .context("collect version probe output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "staged installer version probe exceeded {} seconds and was terminated",
                timeout.as_secs_f32()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        bail!("refusing to overwrite an existing update handoff file");
    }
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn create_managed_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("managed update directory cannot be a symlink");
    }
    Ok(())
}

fn read_and_validate_receipt(
    data_dir: &Path,
    receipt_path: &Path,
    require_current_staged_exe: bool,
) -> Result<UpdateReceipt> {
    let metadata = fs::symlink_metadata(receipt_path).context("inspect update receipt")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("update receipt must be a regular file");
    }
    if metadata.len() == 0 || metadata.len() > MAX_RECEIPT_BYTES {
        bail!("update receipt has an invalid size");
    }
    let bytes = fs::read(receipt_path).context("read update receipt")?;
    let receipt: UpdateReceipt = serde_json::from_slice(&bytes).context("parse update receipt")?;
    validate_receipt_paths(data_dir, receipt_path, &receipt, require_current_staged_exe)?;
    Version::parse(&receipt.expected_version).context("receipt version is invalid")?;
    validate_sha256(&receipt.expected_sha256)?;
    if receipt.parent_pid == 0 || receipt.parent_pid == std::process::id() {
        bail!("receipt parent process is invalid");
    }
    Ok(receipt)
}

fn validate_receipt_paths(
    data_dir: &Path,
    receipt_path: &Path,
    receipt: &UpdateReceipt,
    require_current_staged_exe: bool,
) -> Result<()> {
    let updates_root = data_dir.join("updates");
    let receipts_root = updates_root.join("receipts");
    let staged_root = updates_root.join("staged");
    let installer_root = data_dir.join("installer");
    for directory in [&updates_root, &receipts_root, &staged_root] {
        let metadata = fs::symlink_metadata(directory)
            .with_context(|| format!("inspect {}", directory.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("managed update directory is not a real directory");
        }
    }
    let receipt_parent = receipt_path
        .parent()
        .context("update receipt has no parent")?;
    if fs::canonicalize(receipt_parent)? != fs::canonicalize(&receipts_root)? {
        bail!("update receipt is outside the managed receipt directory");
    }
    let receipt_name = receipt_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("update receipt filename is invalid")?;
    if receipt_name != format!("{}.json", receipt.update_id) {
        bail!("update receipt filename does not match its update ID");
    }

    let expected_stage = staged_root
        .join(receipt.update_id.to_string())
        .join(INSTALLER_ASSET);
    if receipt.staged_path != expected_stage {
        bail!("receipt names an unmanaged staged installer");
    }
    let stage_parent = receipt
        .staged_path
        .parent()
        .context("staged installer has no parent")?;
    let stage_metadata = fs::symlink_metadata(stage_parent)?;
    if !stage_metadata.is_dir() || stage_metadata.file_type().is_symlink() {
        bail!("staged installer directory cannot be a symlink");
    }
    if !fs::canonicalize(stage_parent)?.starts_with(fs::canonicalize(&staged_root)?) {
        bail!("staged installer is outside the managed staging directory");
    }

    let expected_destination = installer_root.join(INSTALLER_ASSET);
    if receipt.managed_destination != expected_destination {
        bail!("receipt names an unmanaged installer destination");
    }
    create_managed_directory(&installer_root)?;
    if fs::canonicalize(
        receipt
            .managed_destination
            .parent()
            .context("managed installer has no parent")?,
    )? != fs::canonicalize(&installer_root)?
    {
        bail!("managed installer destination escaped its directory");
    }
    if require_current_staged_exe
        && fs::canonicalize(std::env::current_exe()?)? != fs::canonicalize(&receipt.staged_path)?
    {
        bail!("update finalizer is not running from the receipt's staged installer");
    }
    Ok(())
}

fn promote_and_launch<F>(receipt: &UpdateReceipt, launch: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let destination = &receipt.managed_destination;
    let parent = destination
        .parent()
        .context("managed installer has no parent")?;
    create_managed_directory(parent)?;
    let temporary = parent.join(format!(
        "BeatblockOnlineInstaller.{}.tmp",
        receipt.update_id
    ));
    let backup = parent.join(format!(
        "BeatblockOnlineInstaller.{}.previous.exe",
        receipt.update_id
    ));
    for path in [&temporary, &backup] {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("installer promotion path cannot be a symlink");
        }
        if fs::symlink_metadata(path).is_ok() {
            bail!("installer promotion handoff already exists");
        }
    }
    let mut source = File::open(&receipt.staged_path).context("open verified installer")?;
    let mut target = File::options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .context("create installer promotion handoff")?;
    std::io::copy(&mut source, &mut target).context("copy verified installer for promotion")?;
    target
        .sync_all()
        .context("flush installer promotion handoff")?;
    drop(target);
    let copied = hex::encode(Sha256::digest(fs::read(&temporary)?));
    if copied != receipt.expected_sha256 {
        let _ = fs::remove_file(&temporary);
        bail!("installer promotion copy failed hash verification");
    }

    let had_previous = destination.is_file();
    if had_previous {
        fs::rename(destination, &backup).context("back up managed installer")?;
    }
    if let Err(error) = fs::rename(&temporary, destination).context("promote managed installer") {
        if had_previous {
            fs::rename(&backup, destination).with_context(|| {
                format!("restore prior managed installer after promotion failed: {error:#}")
            })?;
        }
        return Err(error);
    }
    if let Err(error) = launch(destination) {
        let _ = fs::remove_file(destination);
        if had_previous {
            fs::rename(&backup, destination)
                .context("restore prior managed installer after relaunch failure")?;
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_process_exit(pid: u32) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        let error = unsafe { GetLastError() };
        if error == ERROR_INVALID_PARAMETER {
            // The parent exited between spawning this process and requesting
            // the wait handle, which is already the desired state.
            return Ok(());
        }
        bail!("could not wait for installer process {pid}: Windows error {error}");
    }
    let result = unsafe { WaitForSingleObject(process, 60_000) };
    unsafe { CloseHandle(process) };
    if result != WAIT_OBJECT_0 {
        bail!("timed out waiting for the previous installer to exit");
    }
    Ok(())
}

#[cfg(not(windows))]
fn wait_for_process_exit(_pid: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_self_update_is_rejected_with_recovery_instructions() {
        assert!(reject_elevated_self_update(false).is_ok());
        let error = reject_elevated_self_update(true).unwrap_err().to_string();
        assert!(error.contains("running as administrator"));
        assert!(error.contains("reopen BeatblockOnlineInstaller.exe normally"));
    }

    fn asset(name: &str, size: u64, digest: Option<&str>) -> String {
        format!(
            r#"{{"name":"{name}","browser_download_url":"https://github.com/DupeisTaken/beatblock-online/releases/download/1.0.0/{name}","size":{size},"digest":{}}}"#,
            digest
                .map(|value| format!("\"{value}\""))
                .unwrap_or_else(|| "null".into())
        )
    }

    fn release_json(tag: &str, prerelease: bool, assets: &[String]) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","html_url":"https://github.com/DupeisTaken/beatblock-online/releases/tag/{tag}","draft":false,"prerelease":{prerelease},"assets":[{}]}}]"#,
            assets.join(",")
        )
    }

    fn fake_pe(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x90];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(0x80u32).to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn preview_build_finds_a_complete_newer_preview_release() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let body = release_json(
            "0.3.0-beta.4",
            true,
            &[
                asset(INSTALLER_ASSET, 42, Some(&digest)),
                asset(CHECKSUM_ASSET, 80, None),
            ],
        );
        let check = evaluate_releases("0.3.0-beta.3", &body).unwrap();
        assert!(check.update_available());
        assert_eq!(
            check.latest_version,
            Some(Version::parse("0.3.0-beta.4").unwrap())
        );
        assert!(check.installable_update().is_some());
    }

    #[test]
    fn stable_build_ignores_prereleases_and_reports_current_release() {
        let body = r#"[
            {"tag_name":"1.1.0-beta.1","html_url":"https://example.test/beta","draft":false,"prerelease":true,"assets":[]},
            {"tag_name":"1.0.0","html_url":"https://example.test/stable","draft":false,"prerelease":false,"assets":[]}
        ]"#;
        let check = evaluate_releases("1.0.0", body).unwrap();
        assert!(!check.update_available());
        assert_eq!(check.latest_version, Some(Version::new(1, 0, 0)));
    }

    #[test]
    fn release_selection_uses_semver_instead_of_tag_text_order() {
        let body = r#"[
            {"tag_name":"0.9.9","html_url":"https://example.test/old","draft":false,"prerelease":false,"assets":[]},
            {"tag_name":"0.10.0","html_url":"https://example.test/new","draft":false,"prerelease":false,"assets":[]}
        ]"#;
        let check = evaluate_releases("0.9.0", body).unwrap();
        assert_eq!(check.latest_version, Some(Version::new(0, 10, 0)));
        assert_eq!(
            check.release_url.as_deref(),
            Some("https://example.test/new")
        );
    }

    #[test]
    fn one_historical_v_prefix_remains_readable_but_is_not_part_of_semver() {
        let body = r#"[{
            "tag_name":"v1.0.1",
            "html_url":"https://example.test/legacy",
            "draft":false,
            "prerelease":false,
            "assets":[]
        }]"#;
        let check = evaluate_releases("1.0.0", body).unwrap();
        assert_eq!(check.latest_version, Some(Version::new(1, 0, 1)));

        let malformed = body.replace("v1.0.1", "vv1.0.1");
        let check = evaluate_releases("1.0.0", &malformed).unwrap();
        assert!(check.latest_version.is_none());
    }

    #[test]
    fn incomplete_assets_allow_notes_but_not_self_update() {
        let body = release_json("1.1.0", false, &[asset(CHECKSUM_ASSET, 80, None)]);
        let check = evaluate_releases("1.0.0", &body).unwrap();
        assert!(check.update_available());
        assert!(check.release.is_none());
        assert!(check.status().contains("incomplete"));
    }

    #[test]
    fn duplicate_or_untrusted_assets_are_rejected() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let duplicated = release_json(
            "1.1.0",
            false,
            &[
                asset(INSTALLER_ASSET, 42, Some(&digest)),
                asset(INSTALLER_ASSET, 42, Some(&digest)),
                asset(CHECKSUM_ASSET, 80, None),
            ],
        );
        assert!(evaluate_releases("1.0.0", &duplicated).is_err());
        assert!(validate_download_url("http://github.com/file").is_err());
        assert!(validate_download_url("https://evil.example/file").is_err());
        assert!(trusted_download_url(
            &Url::parse("https://objects.githubusercontent.com/release").unwrap()
        ));
        assert!(!trusted_download_url(
            &Url::parse("http://objects.githubusercontent.com/release").unwrap()
        ));
        let oversized = release_json(
            "1.1.0",
            false,
            &[
                asset(INSTALLER_ASSET, MAX_INSTALLER_BYTES + 1, Some(&digest)),
                asset(CHECKSUM_ASSET, 80, None),
            ],
        );
        assert!(evaluate_releases("1.0.0", &oversized).is_err());
    }

    #[test]
    fn checksum_requires_one_exact_installer_entry() {
        let digest = "a".repeat(64);
        assert_eq!(
            checksum_for_installer(
                format!(
                    "{digest}  {INSTALLER_ASSET}\n{}  other.zip\n",
                    "b".repeat(64)
                )
                .as_bytes()
            )
            .unwrap(),
            digest
        );
        assert!(
            checksum_for_installer(format!("{}  other.exe\n", "a".repeat(64)).as_bytes()).is_err()
        );
        assert!(checksum_for_installer(
            format!("{0}  {1}\n{0} *{1}\n", "a".repeat(64), INSTALLER_ASSET).as_bytes()
        )
        .is_err());
    }

    #[test]
    fn installer_requires_both_release_hash_sources_to_match() {
        let bytes = fake_pe(b"verified");
        let digest = hex::encode(Sha256::digest(&bytes));
        let sums = format!("{digest}  {INSTALLER_ASSET}\n");
        assert_eq!(
            verify_installer_hashes(&bytes, &digest, sums.as_bytes()).unwrap(),
            digest
        );
        assert!(verify_installer_hashes(&bytes, &"a".repeat(64), sums.as_bytes()).is_err());
        let wrong_sums = format!("{}  {INSTALLER_ASSET}\n", "b".repeat(64));
        assert!(verify_installer_hashes(&bytes, &digest, wrong_sums.as_bytes()).is_err());
    }

    #[test]
    fn pe_validation_requires_x64() {
        assert!(validate_pe_x64(&fake_pe(b"payload")).is_ok());
        let mut x86 = fake_pe(b"payload");
        x86[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        assert!(validate_pe_x64(&x86).is_err());
        assert!(validate_pe_x64(b"not-pe").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn subprocess_output_is_bounded_and_reaps_a_wedged_probe() {
        let mut quick = Command::new("cmd");
        quick.args(["/C", "echo", "ready"]);
        let output = command_output_with_timeout(quick, Duration::from_secs(2)).unwrap();
        assert!(output.status.success());

        let mut slow = Command::new("cmd");
        slow.args(["/C", "ping", "-n", "6", "127.0.0.1"]);
        let started = Instant::now();
        let error = command_output_with_timeout(slow, Duration::from_millis(100)).unwrap_err();
        assert!(error.to_string().contains("terminated"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn receipt_confines_destination_and_binds_id_hash_version_and_pid() {
        let root = std::env::temp_dir().join(format!("bbt-update-{}", Uuid::new_v4()));
        let id = Uuid::new_v4();
        let receipts = root.join("updates/receipts");
        let staged_dir = root.join("updates/staged").join(id.to_string());
        fs::create_dir_all(&receipts).unwrap();
        fs::create_dir_all(&staged_dir).unwrap();
        fs::create_dir_all(root.join("installer")).unwrap();
        let staged_path = staged_dir.join(INSTALLER_ASSET);
        fs::write(&staged_path, fake_pe(b"safe")).unwrap();
        let digest = hex::encode(Sha256::digest(fs::read(&staged_path).unwrap()));
        let mut receipt = UpdateReceipt {
            update_id: id,
            expected_version: "1.2.3".into(),
            expected_sha256: digest,
            staged_path,
            managed_destination: root.join("installer").join(INSTALLER_ASSET),
            parent_pid: std::process::id().saturating_add(1),
        };
        let path = receipts.join(format!("{id}.json"));
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(read_and_validate_receipt(&root, &path, false).is_ok());

        receipt.managed_destination = root.join("external-launcher.exe");
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(read_and_validate_receipt(&root, &path, false).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn relaunch_failure_restores_previous_managed_installer() {
        let root = std::env::temp_dir().join(format!("bbt-promote-{}", Uuid::new_v4()));
        let staged_dir = root.join("updates/staged/id");
        let installer_dir = root.join("installer");
        fs::create_dir_all(&staged_dir).unwrap();
        fs::create_dir_all(&installer_dir).unwrap();
        let staged = staged_dir.join(INSTALLER_ASSET);
        fs::write(&staged, b"new verified installer").unwrap();
        let destination = installer_dir.join(INSTALLER_ASSET);
        fs::write(&destination, b"prior managed installer").unwrap();
        let receipt = UpdateReceipt {
            update_id: Uuid::new_v4(),
            expected_version: "1.2.3".into(),
            expected_sha256: hex::encode(Sha256::digest(b"new verified installer")),
            staged_path: staged,
            managed_destination: destination.clone(),
            parent_pid: 1,
        };
        let error =
            promote_and_launch(&receipt, |_| bail!("simulated launch failure")).unwrap_err();
        assert!(error.to_string().contains("simulated"));
        assert_eq!(fs::read(&destination).unwrap(), b"prior managed installer");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_update_cleanup_removes_only_id_bound_handoffs() {
        let root = std::env::temp_dir().join(format!("bbt-cleanup-{}", Uuid::new_v4()));
        let id = Uuid::new_v4();
        let stage = root.join("updates/staged").join(id.to_string());
        let receipts = root.join("updates/receipts");
        let installer = root.join("installer");
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir_all(&receipts).unwrap();
        fs::create_dir_all(&installer).unwrap();
        fs::write(stage.join(INSTALLER_ASSET), b"staged").unwrap();
        fs::write(receipts.join(format!("{id}.json")), b"receipt").unwrap();
        fs::write(
            installer.join(format!("BeatblockOnlineInstaller.{id}.previous.exe")),
            b"prior",
        )
        .unwrap();
        let unrelated = installer.join("keep-me.exe");
        fs::write(&unrelated, b"unrelated").unwrap();

        cleanup_completed_update(&root, id, u32::MAX).unwrap();
        assert!(!stage.exists());
        assert!(!receipts.join(format!("{id}.json")).exists());
        assert!(unrelated.is_file());
        // A relaunched managed copy may encounter cleanup already performed by
        // antivirus or a prior retry. Missing ID-bound artifacts are success.
        cleanup_completed_update(&root, id, u32::MAX).unwrap();
        assert!(unrelated.is_file());
        let _ = fs::remove_dir_all(root);
    }
}
