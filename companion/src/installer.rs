use crate::mod_payload::SHARED_MOD_PAYLOAD;
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::RwLock,
    time::{Duration, Instant},
};

static LOVELY_PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lovely-version.dll"));
static OBS_PLUGIN_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/beatblock-online-obs.dll"));
static RUNTIME_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/BeatblockOnlineRuntime.exe"));
const RUNTIME_FILE_NAME: &str = "BeatblockOnlineRuntime.exe";
const MAX_INSTALL_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_OBS_MARKER_BYTES: u64 = 64 * 1024;
// Keep upgrade cleanup compatible without presenting the retired product name
// as a current executable anywhere in the installer UI or documentation.
const LEGACY_RUNTIME_FILE_NAME: &str = concat!("Beatblock", "TogetherRuntime.exe");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    Standalone,
    BeatblockPlus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallManifest {
    pub version: String,
    pub game_directory: PathBuf,
    pub mods_directory: PathBuf,
    pub distribution: Distribution,
    pub installed_files: Vec<PathBuf>,
    pub lovely_owned: bool,
    pub lovely_backup: Option<PathBuf>,
    /// Isolated game copies need a persistent Steam app id when launched
    /// directly from Explorer. Steam-managed copies obtain it from Steam.
    #[serde(default)]
    pub steam_app_id_owned: bool,
    #[serde(default)]
    pub steam_app_id_backup: Option<PathBuf>,
    #[serde(default)]
    pub runtime_path: Option<PathBuf>,
    #[serde(default)]
    pub maintenance_installer: Option<PathBuf>,
    #[serde(default)]
    pub firewall_installed: bool,
    /// The profile choice is retained so Repair recreates the same rule rather
    /// than silently falling back to the private-only default.
    #[serde(default)]
    pub firewall_public: bool,
    /// Hashes are relative to the shared Mods directory and allow inspection
    /// to distinguish a working installation from a stale manifest.
    #[serde(default)]
    pub file_hashes: std::collections::BTreeMap<PathBuf, String>,
    #[serde(default)]
    pub lovely_original_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObsInstallManifest {
    version: String,
    obs_directory: PathBuf,
    plugin: PathBuf,
    locale: PathBuf,
    plugin_sha256: String,
}

struct ObsInstallTransaction {
    plugin: PathBuf,
    plugin_rollback: FileRollback,
    locale_rollback: FileRollback,
    marker_rollback: FileRollback,
    legacy_rollbacks: Vec<FileRollback>,
}

impl ObsInstallTransaction {
    fn commit(mut self) -> PathBuf {
        self.plugin_rollback.commit();
        self.locale_rollback.commit();
        self.marker_rollback.commit();
        for rollback in &mut self.legacy_rollbacks {
            rollback.commit();
        }
        self.plugin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Install,
    Repair,
    Restore,
    Uninstall,
    Launch,
    Inspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgress {
    pub operation: OperationKind,
    pub phase: String,
    pub percent: u8,
    pub message: String,
    pub severity: Severity,
    pub terminal: bool,
}

impl OperationProgress {
    fn step(
        operation: OperationKind,
        phase: &str,
        percent: u8,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            phase: phase.into(),
            percent,
            message: message.into(),
            severity: Severity::Info,
            terminal: false,
        }
    }
    fn complete(operation: OperationKind, message: impl Into<String>) -> Self {
        Self {
            operation,
            phase: "complete".into(),
            percent: 100,
            message: message.into(),
            severity: Severity::Success,
            terminal: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Ready,
    Attention,
    Optional,
    Missing,
    Broken,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    pub name: String,
    pub state: ComponentState,
    pub label: String,
    pub included: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInspection {
    pub game_directory: PathBuf,
    pub valid: bool,
    pub compatible_layout: bool,
    pub distribution: Distribution,
    pub install_state: String,
    pub managed_elsewhere: Option<PathBuf>,
    pub repair_required: bool,
    pub components: Vec<ComponentStatus>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchReport {
    pub executable: PathBuf,
    pub log_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStatus {
    pub game_directory: Option<PathBuf>,
    pub installed: bool,
    pub distribution: Option<Distribution>,
    pub lovely_present: bool,
    pub beatblock_plus_present: bool,
    pub runtime_present: bool,
    pub obs_plugin_present: bool,
    pub compatible_layout: bool,
    pub runtime_bundled: bool,
    pub lovely_bundled: bool,
    pub obs_plugin_bundled: bool,
    pub firewall_installed: bool,
    pub message: String,
}

pub struct Installer {
    data_dir: PathBuf,
    mods_directory_override: Option<PathBuf>,
    // The GUI can point at a portable or custom OBS copy. Keep the raw
    // candidate so an invalid manual choice cannot silently fall back to a
    // different installation while the user believes the chosen one is used.
    obs_directory_override: RwLock<Option<PathBuf>>,
    // Fixture-backed installer tests must never depend on an unrelated OBS
    // process running on the developer's workstation. Production builds do
    // not contain this override, so the real process lock remains mandatory.
    #[cfg(test)]
    obs_running_override: Option<bool>,
}

impl Installer {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            mods_directory_override: None,
            obs_directory_override: RwLock::new(None),
            #[cfg(test)]
            obs_running_override: None,
        }
    }

    #[cfg(test)]
    fn with_mods_directory(data_dir: PathBuf, mods_directory: PathBuf) -> Self {
        Self {
            data_dir,
            mods_directory_override: Some(mods_directory),
            obs_directory_override: RwLock::new(None),
            obs_running_override: Some(false),
        }
    }

    fn detected_obs_running(&self) -> Result<bool> {
        #[cfg(test)]
        if let Some(running) = self.obs_running_override {
            return Ok(running);
        }
        obs_is_running()
    }

    /// Selects an OBS root (or a folder/executable beneath that root). The
    /// candidate remains explicit even when invalid so availability feedback
    /// describes this selection instead of an unrelated automatic match.
    pub fn set_obs_directory(&self, directory: Option<PathBuf>) -> Result<()> {
        *self
            .obs_directory_override
            .write()
            .map_err(|_| anyhow::anyhow!("OBS location selection lock is poisoned"))? = directory;
        Ok(())
    }

    /// Returns the validated OBS root used by installation. A prior successful
    /// custom install is reused before standard-location discovery.
    pub fn obs_directory(&self) -> Option<PathBuf> {
        let selected = self
            .obs_directory_override
            .read()
            .ok()
            .and_then(|directory| directory.clone());
        if let Some(selected) = selected {
            return normalize_obs_directory(&selected);
        }
        self.recorded_obs_directory().or_else(detect_obs_directory)
    }

    fn recorded_obs_directory(&self) -> Option<PathBuf> {
        let bytes = read_bounded_file(
            &self.data_dir.join("obs-install.json"),
            MAX_OBS_MARKER_BYTES,
        )
        .ok()?;
        let record = serde_json::from_slice::<ObsInstallManifest>(&bytes).ok()?;
        normalize_obs_directory(&record.obs_directory)
    }

    fn mods_directory(&self) -> Option<PathBuf> {
        self.mods_directory_override
            .clone()
            .or_else(default_mods_directory)
    }

    /// Initial selection order is deliberately stable: the user's explicit
    /// field wins in the UI, then the managed manifest, then Steam discovery.
    pub fn initial_game_directory(&self) -> Option<PathBuf> {
        self.load_manifest()
            .ok()
            .flatten()
            .map(|m| m.game_directory)
            .or_else(|| self.find_game_directory())
    }

    pub fn inspect_target(&self, selected: &Path) -> TargetInspection {
        let manifest = self.load_manifest().ok().flatten();
        let valid_result = validate_game_directory(selected);
        let valid = valid_result.is_ok();
        let validation_message = valid_result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        // Exact identity comes from Beatblock's own displayed build token once
        // the game starts. Newer structurally valid releases are installable
        // without publishing a new installer allowlist.
        let compatible_layout = valid;
        let mods_directory = self
            .mods_directory()
            .unwrap_or_else(|| self.data_dir.join("Mods"));
        let plus = find_beatblock_plus(&mods_directory);
        let distribution = manifest
            .as_ref()
            .filter(|m| m.game_directory == selected)
            .map(|m| m.distribution)
            .unwrap_or(if plus {
                Distribution::BeatblockPlus
            } else {
                Distribution::Standalone
            });
        let managed_here = manifest
            .as_ref()
            .is_some_and(|m| m.game_directory == selected);
        let managed_elsewhere = manifest
            .as_ref()
            .filter(|m| m.game_directory != selected)
            .map(|m| m.game_directory.clone());
        let mod_dir = mods_directory.join("BeatblockOnline");
        let shared_ok = SHARED_MOD_PAYLOAD
            .iter()
            .all(|(relative, bytes)| file_matches(&mod_dir.join(relative), bytes));
        let adapter_ok = match distribution {
            Distribution::Standalone => file_matches(
                &mod_dir.join("lovely/bootstrap.toml"),
                include_bytes!("../../mod/standalone/lovely/bootstrap.toml"),
            ),
            Distribution::BeatblockPlus => {
                ["mod.json", "main.lua", "config.lua", "states/Online.lua"]
                    .iter()
                    .all(|p| mod_dir.join(p).is_file())
            }
        };
        let lovely_path = selected.join("version.dll");
        let lovely_present = lovely_path.is_file();
        let lovely_matches =
            !LOVELY_PAYLOAD.is_empty() && file_matches(&lovely_path, LOVELY_PAYLOAD);
        let lovely_compatible = is_lovely_injector(&lovely_path);
        let steam_managed = self.is_steam_managed_game_directory(selected);
        let app_id_path = selected.join("steam_appid.txt");
        let direct_launch_ready = steam_managed || file_matches(&app_id_path, b"3045200\n");
        let runtime_present = manifest
            .as_ref()
            .and_then(|m| m.runtime_path.as_ref())
            .is_some_and(|p| p.is_file() && file_matches(p, RUNTIME_PAYLOAD));
        let renderer_ready = self
            .data_dir
            .join("renderer-profile/Beatblock/Mods/BeatblockOnlineRenderer/bbt/dashboard_model.lua")
            .is_file();
        let backup_warning = manifest
            .as_ref()
            .and_then(|m| m.lovely_backup.as_ref())
            .is_some_and(|p| {
                p.is_file()
                    && lovely_path.is_file()
                    && sha256_file(p).ok() == sha256_file(&lovely_path).ok()
            });
        let required_ready = valid
            && managed_here
            && adapter_ok
            && shared_ok
            && lovely_compatible
            && runtime_present
            && direct_launch_ready;
        let repair_required = managed_here && !required_ready;
        let state = if !valid {
            "INVALID TARGET"
        } else if managed_elsewhere.is_some() {
            "MOVE INSTALLATION"
        } else if repair_required {
            "REPAIR REQUIRED"
        } else if managed_here {
            "READY"
        } else {
            "NOT INSTALLED"
        };
        let mut components = Vec::new();
        components.push(component(
            "Game build",
            if !valid {
                ComponentState::Broken
            } else {
                ComponentState::Ready
            },
            if !valid { "Invalid" } else { "Compatible" },
            "—",
            if valid {
                "Supported Beatblock layout; exact build identity is checked in-game".into()
            } else {
                validation_message.clone()
            },
        ));
        components.push(component(
            "In-game adapter",
            state_for(managed_here, adapter_ok),
            if adapter_ok { "Installed" } else { "Broken" },
            "Yes",
            distribution_label(distribution).to_string(),
        ));
        components.push(component(
            "Shared Lua payload",
            state_for(managed_here, shared_ok),
            if shared_ok {
                "Installed"
            } else if managed_here {
                "Broken"
            } else {
                "Not installed"
            },
            "Yes",
            if shared_ok {
                "All Lovely modules verified"
            } else {
                "Required module is missing or changed"
            },
        ));
        let lovely_state = if !lovely_present {
            ComponentState::Missing
        } else if lovely_matches || lovely_compatible {
            if backup_warning {
                ComponentState::Attention
            } else {
                ComponentState::Ready
            }
        } else {
            ComponentState::Attention
        };
        components.push(component(
            "Lovely injector",
            lovely_state,
            if !lovely_present {
                "Missing"
            } else if backup_warning {
                "Attention"
            } else if lovely_matches {
                "Installed"
            } else if lovely_compatible {
                "Compatible"
            } else {
                "Invalid"
            },
            "Yes",
            if backup_warning {
                "Legacy backup matches injector; backup preserved"
            } else if lovely_matches {
                "Bundled no-console build"
            } else if lovely_compatible {
                "Existing compatible Lovely build will be preserved"
            } else {
                "Existing file is not a compatible Lovely injector"
            },
        ));
        components.push(component(
            "Direct launch",
            if direct_launch_ready {
                ComponentState::Ready
            } else if managed_here {
                ComponentState::Broken
            } else {
                ComponentState::NotInstalled
            },
            if steam_managed {
                "Steam managed"
            } else if direct_launch_ready {
                "Ready"
            } else {
                "Not configured"
            },
            "Automatic",
            if steam_managed {
                "Steam supplies app id 3045200"
            } else if direct_launch_ready {
                "Beatblock.exe can be opened directly"
            } else {
                "Installer will add reversible isolated-copy launch support"
            },
        ));
        components.push(component(
            "Hidden runtime",
            state_for(managed_here, runtime_present),
            if runtime_present {
                "Installed"
            } else if managed_here {
                "Broken"
            } else {
                "Not installed"
            },
            "Yes",
            if runtime_present {
                env!("CARGO_PKG_VERSION")
            } else {
                "Runtime hash does not match"
            },
        ));
        components.push(component(
            "Renderer payload",
            if renderer_ready {
                ComponentState::Ready
            } else if managed_here {
                ComponentState::Attention
            } else {
                ComponentState::NotInstalled
            },
            if renderer_ready {
                "Installed"
            } else {
                "Not prepared"
            },
            "Yes",
            if renderer_ready {
                "Spectator profile ready"
            } else {
                "Created when repaired or first used"
            },
        ));
        let obs = self.obs_plugin_ready();
        let obs_recorded = self.data_dir.join("obs-install.json").is_file();
        components.push(component(
            "OBS video/audio plugin",
            if obs {
                ComponentState::Ready
            } else if obs_recorded {
                ComponentState::Broken
            } else {
                ComponentState::Optional
            },
            if obs {
                "Installed"
            } else if obs_recorded {
                "Broken"
            } else {
                "Optional"
            },
            "Conditional",
            if obs {
                "Installed and hash verified"
            } else if OBS_PLUGIN_PAYLOAD.is_empty() {
                "Not included in this build"
            } else if obs_recorded {
                "Installed files failed hash verification"
            } else if self.obs_directory().is_none() {
                "OBS Studio was not detected"
            } else {
                "OBS 32 video/audio sources are available to install"
            },
        ));
        let firewall = manifest.as_ref().is_some_and(|m| m.firewall_installed);
        components.push(component(
            "Firewall rule",
            if firewall {
                ComponentState::Ready
            } else if managed_here {
                ComponentState::Attention
            } else {
                ComponentState::NotInstalled
            },
            if firewall { "Installed" } else { "Missing" },
            "Yes",
            "Program-scoped QUIC hosting rule",
        ));
        TargetInspection {
            game_directory: selected.to_owned(),
            valid,
            compatible_layout,
            distribution,
            install_state: state.into(),
            managed_elsewhere,
            repair_required,
            components,
            message: if valid {
                "Compatible Beatblock layout detected. Newer game builds are accepted but unverified; exact room matching happens after Beatblock starts.".into()
            } else {
                validation_message
            },
        }
    }

    pub fn detect(&self) -> InstallStatus {
        let game_directory = self.initial_game_directory();
        let mods_directory = self.mods_directory();
        let manifest = self.load_manifest().ok().flatten();
        let lovely_present = game_directory
            .as_ref()
            .is_some_and(|path| path.join("version.dll").is_file());
        let beatblock_plus_present = mods_directory
            .as_ref()
            .is_some_and(|path| find_beatblock_plus(path));
        let compatible_layout = game_directory
            .as_ref()
            .is_some_and(|path| validate_game_directory(path).is_ok());
        let runtime_present = manifest
            .as_ref()
            .and_then(|value| value.runtime_path.as_ref())
            .is_some_and(|path| path.is_file());
        let obs_plugin_present = self.obs_plugin_ready();
        InstallStatus {
            game_directory: game_directory.clone(),
            installed: manifest.is_some(),
            distribution: manifest.as_ref().map(|manifest| manifest.distribution),
            lovely_present,
            beatblock_plus_present,
            runtime_present,
            obs_plugin_present,
            compatible_layout,
            runtime_bundled: !RUNTIME_PAYLOAD.is_empty(),
            lovely_bundled: !LOVELY_PAYLOAD.is_empty(),
            obs_plugin_bundled: !OBS_PLUGIN_PAYLOAD.is_empty(),
            firewall_installed: manifest
                .as_ref()
                .is_some_and(|value| value.firewall_installed),
            message: if compatible_layout {
                "Compatible Beatblock layout detected; the running game reports its exact build"
                    .into()
            } else {
                "Beatblock was not found or its required game files are missing".into()
            },
        }
    }

    pub fn install(&self, explicit_game_directory: Option<PathBuf>) -> Result<InstallManifest> {
        self.install_with_options(explicit_game_directory, false)
    }

    pub fn install_with_options(
        &self,
        explicit_game_directory: Option<PathBuf>,
        allow_unknown_build: bool,
    ) -> Result<InstallManifest> {
        self.install_with_distribution(explicit_game_directory, allow_unknown_build, None)
    }

    pub fn install_with_distribution(
        &self,
        explicit_game_directory: Option<PathBuf>,
        allow_unknown_build: bool,
        requested_distribution: Option<Distribution>,
    ) -> Result<InstallManifest> {
        self.install_with_progress(
            explicit_game_directory,
            allow_unknown_build,
            requested_distribution,
            |_| {},
        )
    }

    pub fn install_with_progress<F>(
        &self,
        explicit_game_directory: Option<PathBuf>,
        allow_unknown_build: bool,
        requested_distribution: Option<Distribution>,
        progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        self.install_with_progress_options(
            explicit_game_directory,
            allow_unknown_build,
            requested_distribution,
            false,
            progress,
        )
    }

    /// Installs every managed component in one transaction. Keeping the
    /// firewall profile in this operation avoids a second privileged pass that
    /// can turn an otherwise successful installation into a false failure.
    pub fn install_with_progress_options<F>(
        &self,
        explicit_game_directory: Option<PathBuf>,
        allow_unknown_build: bool,
        requested_distribution: Option<Distribution>,
        firewall_public: bool,
        progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        self.install_with_progress_platform(
            explicit_game_directory,
            allow_unknown_build,
            requested_distribution,
            firewall_public,
            true,
            progress,
        )
    }

    pub fn installed_options(&self) -> Option<(Distribution, bool)> {
        self.load_manifest()
            .ok()
            .flatten()
            .map(|manifest| (manifest.distribution, manifest.firewall_public))
    }

    /// Stages the optional OBS source before changing the game, then commits it
    /// only after the core transaction verifies. A failure in either component
    /// therefore restores OBS and leaves no misleading partial-success state.
    pub fn install_with_optional_obs<F>(
        &self,
        explicit_game_directory: Option<PathBuf>,
        allow_unknown_build: bool,
        requested_distribution: Option<Distribution>,
        firewall_public: bool,
        install_obs: bool,
        mut progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        if beatblock_is_running()
            .context("could not inspect running Beatblock processes before installation")?
        {
            bail!(
                "Beatblock is running. Close Beatblock and refresh the installer before changing game files"
            );
        }
        let obs_transaction = if install_obs {
            progress(OperationProgress::step(
                OperationKind::Install,
                "optional_components",
                2,
                "Staging and verifying the OBS 32 video/audio sources",
            ));
            Some(self.stage_obs_plugin()?)
        } else {
            None
        };
        let manifest = self.install_with_progress_options(
            explicit_game_directory,
            allow_unknown_build,
            requested_distribution,
            firewall_public,
            &mut progress,
        )?;
        if let Some(transaction) = obs_transaction {
            transaction.commit();
        }
        Ok(manifest)
    }

    /// Runs the file transaction with optional Windows integration. Production
    /// callers always enable platform changes; disabling them lets tests cover
    /// the real staging, swapping, hashing, backup, move, and manifest logic
    /// without modifying the developer's firewall or uninstall registry.
    fn install_with_progress_platform<F>(
        &self,
        explicit_game_directory: Option<PathBuf>,
        _allow_unknown_build: bool,
        requested_distribution: Option<Distribution>,
        firewall_public: bool,
        apply_platform_changes: bool,
        mut progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        if beatblock_is_running()
            .context("could not inspect running Beatblock processes before installation")?
        {
            bail!(
                "Beatblock is running. Close Beatblock and refresh the installer before changing game files"
            );
        }
        progress(OperationProgress::step(
            OperationKind::Install,
            "validation",
            3,
            "Validating the selected Beatblock folder",
        ));
        if RUNTIME_PAYLOAD.is_empty() {
            bail!("this installer build does not contain BeatblockOnlineRuntime.exe");
        }
        let game_directory = explicit_game_directory
            .or_else(|| self.initial_game_directory())
            .context("Beatblock was not found; choose the folder containing Beatblock.exe")?;
        validate_game_directory(&game_directory)?;
        let managed_manifest = self.load_manifest()?;
        let mods_directory = self
            .mods_directory()
            .context("Windows APPDATA is unavailable")?;
        std::fs::create_dir_all(&mods_directory)?;
        progress(OperationProgress::step(
            OperationKind::Install,
            "validation",
            10,
            "Checking adapter compatibility and existing installation",
        ));
        let detected_plus = find_beatblock_plus(&mods_directory);
        let distribution = requested_distribution.unwrap_or(if detected_plus {
            Distribution::BeatblockPlus
        } else {
            Distribution::Standalone
        });
        if distribution == Distribution::BeatblockPlus && !detected_plus {
            bail!("BeatblockPlus installation method was selected, but BeatblockPlus 2.x was not detected");
        }
        if distribution == Distribution::Standalone && detected_plus {
            bail!("Standalone Lovely was selected, but BeatblockPlus 2.x is installed; choose the BeatblockPlus adapter to avoid loading both BBT adapters");
        }
        let mod_directory = mods_directory.join("BeatblockOnline");
        let legacy_mod_directory = mods_directory.join("BeatblockTogether");
        let stage_directory =
            mods_directory.join(format!(".BeatblockOnline.stage-{}", uuid::Uuid::new_v4()));
        let rollback_directory = mods_directory.join(format!(
            ".BeatblockOnline.rollback-{}",
            uuid::Uuid::new_v4()
        ));
        let legacy_rollback_directory = mods_directory.join(format!(
            ".BeatblockTogether.rollback-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(stage_directory.join("bbt"))?;
        std::fs::create_dir_all(stage_directory.join("lovely"))?;
        let mut installed_files = Vec::new();
        let runtime_path = self.data_dir.join("runtime").join(RUNTIME_FILE_NAME);
        progress(OperationProgress::step(
            OperationKind::Install,
            "runtime",
            20,
            "Staging the hidden online runtime",
        ));
        let mut runtime_rollback = FileRollback::replace(&runtime_path, RUNTIME_PAYLOAD)
            .context("install hidden runtime; exit Online before updating")?;
        // Alpha builds used a different runtime filename. Remove every prior
        // managed path transactionally so upgrades cannot leave a launchable
        // stale binary behind; any later install failure restores it.
        let mut previous_runtime_paths =
            vec![self.data_dir.join("runtime").join(LEGACY_RUNTIME_FILE_NAME)];
        if let Some(previous_runtime) = managed_manifest
            .as_ref()
            .and_then(|manifest| manifest.runtime_path.as_ref())
            .filter(|path| *path != &runtime_path)
        {
            if !previous_runtime_paths.contains(previous_runtime) {
                previous_runtime_paths.push(previous_runtime.clone());
            }
        }
        let mut previous_runtime_rollbacks = Vec::new();
        for previous_runtime in previous_runtime_paths {
            if previous_runtime.is_file() {
                previous_runtime_rollbacks.push(
                    FileRollback::remove(&previous_runtime).with_context(|| {
                        format!(
                            "remove the previous online runtime at {}; exit Online before updating",
                            previous_runtime.display()
                        )
                    })?,
                );
            }
        }
        installed_files.push(runtime_path.clone());
        write(
            &stage_directory.join("runtime-path.txt"),
            runtime_path.to_string_lossy().as_bytes(),
            &mut installed_files,
        )?;
        progress(OperationProgress::step(
            OperationKind::Install,
            "mod_payload",
            35,
            "Staging shared Lua modules",
        ));
        for (relative, bytes) in SHARED_MOD_PAYLOAD {
            write(&stage_directory.join(relative), bytes, &mut installed_files)?;
        }
        match distribution {
            Distribution::Standalone => {
                write(
                    &stage_directory.join("lovely/bootstrap.toml"),
                    include_bytes!("../../mod/standalone/lovely/bootstrap.toml"),
                    &mut installed_files,
                )?;
                write(
                    &stage_directory.join("README.txt"),
                    b"Beatblock Online standalone Lovely package. Installed by BeatblockOnlineInstaller.exe.\n",
                    &mut installed_files,
                )?;
            }
            Distribution::BeatblockPlus => {
                write(
                    &stage_directory.join("mod.json"),
                    include_bytes!("../../mod/beatblock-plus/mod.json"),
                    &mut installed_files,
                )?;
                write(
                    &stage_directory.join("main.lua"),
                    include_bytes!("../../mod/beatblock-plus/main.lua"),
                    &mut installed_files,
                )?;
                write(
                    &stage_directory.join("config.lua"),
                    include_bytes!("../../mod/beatblock-plus/config.lua"),
                    &mut installed_files,
                )?;
                std::fs::create_dir_all(stage_directory.join("states"))?;
                write(
                    &stage_directory.join("states/Online.lua"),
                    include_bytes!("../../mod/beatblock-plus/states/Online.lua"),
                    &mut installed_files,
                )?;
            }
        }

        validate_staged_payload(&stage_directory, distribution)?;
        progress(OperationProgress::step(
            OperationKind::Install,
            "mod_payload",
            50,
            "Replacing the mod atomically",
        ));
        let mut mod_rollback =
            DirectoryRollback::activate(&stage_directory, &mod_directory, &rollback_directory)?;
        // Retired alpha installers used a second Lovely mod with the same hook
        // priority. Leaving it in place starts the stale runtime nondeterministically,
        // so quarantine only a positively identified installer-owned copy.
        let mut legacy_mod_rollback = if is_managed_legacy_mod(&legacy_mod_directory) {
            Some(DirectoryRemovalRollback::quarantine(
                &legacy_mod_directory,
                &legacy_rollback_directory,
            )?)
        } else {
            None
        };
        // Convert staged paths to the final destination before they are stored.
        for path in &mut installed_files {
            if let Ok(relative) = path.strip_prefix(&stage_directory) {
                *path = mod_directory.join(relative);
            }
        }

        let lovely_target = game_directory.join("version.dll");
        progress(OperationProgress::step(
            OperationKind::Install,
            "lovely",
            62,
            "Verifying the Lovely injector and backup",
        ));
        let previous = managed_manifest
            .clone()
            .filter(|m| m.game_directory == game_directory);
        let mut lovely_owned = previous.as_ref().is_some_and(|m| m.lovely_owned);
        let mut lovely_backup = previous.as_ref().and_then(|m| m.lovely_backup.clone());
        let mut lovely_original_sha256 = previous
            .as_ref()
            .and_then(|m| m.lovely_original_sha256.clone());
        let mut lovely_rollback = None;
        let bundled_matches =
            !LOVELY_PAYLOAD.is_empty() && file_matches(&lovely_target, LOVELY_PAYLOAD);
        // Preserve a recoverable copy of every pre-existing third-party
        // injector, including source-only validation builds that intentionally
        // run before the bundled release payload has been assembled.
        if lovely_target.is_file()
            && lovely_backup.as_ref().is_none_or(|p| !p.is_file())
            && !bundled_matches
        {
            let backup = self
                .data_dir
                .join("backups")
                .join(backup_name_for(&game_directory));
            if let Some(parent) = backup.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&lovely_target, &backup)?;
            lovely_original_sha256 = sha256_file(&backup).ok();
            lovely_backup = Some(backup);
        } else if !lovely_target.is_file() && lovely_backup.is_none() {
            lovely_owned = true;
        }
        if !LOVELY_PAYLOAD.is_empty() {
            // A compatible third-party Lovely build may already be approved by
            // Windows Application Control. Preserve it unless an earlier
            // manifest proves this installer owns the file and must repair it.
            let preserve_existing = lovely_target.is_file()
                && !bundled_matches
                && !lovely_owned
                && is_lovely_injector(&lovely_target);
            if preserve_existing {
                progress(OperationProgress::step(
                    OperationKind::Install,
                    "lovely",
                    66,
                    "Keeping the existing compatible Lovely injector",
                ));
            } else {
                lovely_rollback = Some(FileRollback::replace(&lovely_target, LOVELY_PAYLOAD)?);
                installed_files.push(lovely_target.clone());
            }
        } else if !lovely_target.is_file() {
            bail!("the release is missing its bundled no-console Lovely payload");
        }

        let app_id_target = game_directory.join("steam_appid.txt");
        let steam_managed = self.is_steam_managed_game_directory(&game_directory);
        let mut steam_app_id_owned = previous.as_ref().is_some_and(|m| m.steam_app_id_owned);
        let mut steam_app_id_backup = previous
            .as_ref()
            .and_then(|m| m.steam_app_id_backup.clone());
        let mut app_id_rollback = None;
        if steam_managed {
            // Never introduce a local app-id override into a Steam library.
            if steam_app_id_owned {
                app_id_rollback = Some(
                    if let Some(backup) = steam_app_id_backup.as_ref().filter(|path| path.is_file())
                    {
                        FileRollback::replace(&app_id_target, &std::fs::read(backup)?)?
                    } else {
                        FileRollback::remove(&app_id_target)?
                    },
                );
            }
            steam_app_id_owned = false;
            steam_app_id_backup = None;
        } else if !file_matches(&app_id_target, b"3045200\n") {
            if app_id_target.is_file()
                && !steam_app_id_owned
                && steam_app_id_backup
                    .as_ref()
                    .is_none_or(|path| !path.is_file())
            {
                let backup = self
                    .data_dir
                    .join("backups")
                    .join(app_id_backup_name_for(&game_directory));
                if let Some(parent) = backup.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&app_id_target, &backup)?;
                steam_app_id_backup = Some(backup);
            }
            app_id_rollback = Some(FileRollback::replace(&app_id_target, b"3045200\n")?);
            steam_app_id_owned = true;
            installed_files.push(app_id_target.clone());
        }

        let maintenance_installer = self.data_dir.join("installer/BeatblockOnlineInstaller.exe");
        if let Some(parent) = maintenance_installer.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let current_exe = std::env::current_exe()?;
        let mut maintenance_rollback = if current_exe != maintenance_installer {
            Some(FileRollback::replace(
                &maintenance_installer,
                &std::fs::read(&current_exe)?,
            )?)
        } else {
            None
        };
        write(
            &mod_directory.join("installer-path.txt"),
            maintenance_installer.to_string_lossy().as_bytes(),
            &mut installed_files,
        )?;
        let firewall_rule_current = managed_manifest.as_ref().is_some_and(|manifest| {
            manifest.firewall_installed
                && manifest.firewall_public == firewall_public
                && manifest.runtime_path.as_ref() == Some(&runtime_path)
        });
        progress(OperationProgress::step(
            OperationKind::Install,
            "system_changes",
            76,
            if apply_platform_changes && firewall_rule_current {
                "Keeping the existing program-scoped firewall rule"
            } else if apply_platform_changes {
                "Applying the program-scoped firewall rule"
            } else {
                "Skipping external Windows changes for isolated verification"
            },
        ));
        if apply_platform_changes && !firewall_rule_current {
            Self::configure_firewall(&runtime_path, firewall_public, true)?;
        }
        let firewall_installed = apply_platform_changes;
        self.prepare_renderer_profile()?;
        progress(OperationProgress::step(
            OperationKind::Install,
            "verification",
            90,
            "Verifying every installed component",
        ));
        let file_hashes = collect_managed_hashes(&mod_directory)?;
        let manifest = InstallManifest {
            version: env!("CARGO_PKG_VERSION").into(),
            game_directory: game_directory.clone(),
            mods_directory,
            distribution,
            installed_files,
            lovely_owned,
            lovely_backup,
            steam_app_id_owned,
            steam_app_id_backup,
            runtime_path: Some(runtime_path.clone()),
            maintenance_installer: Some(maintenance_installer),
            firewall_installed,
            firewall_public,
            file_hashes,
            lovely_original_sha256,
        };
        if apply_platform_changes {
            self.register_uninstall(&manifest)?;
        }
        validate_staged_payload(&mod_directory, distribution)?;
        if !file_matches(&runtime_path, RUNTIME_PAYLOAD)
            || !is_lovely_injector(&lovely_target)
            || (!steam_managed && !file_matches(&app_id_target, b"3045200\n"))
        {
            bail!("post-install verification failed: runtime, Lovely, or direct-launch support is invalid");
        }
        // A move restores the former target only after the new target passes
        // every check. Keep a rollback guard until the new manifest is durable.
        let mut previous_target_rollbacks = Vec::new();
        if let Some(old) = managed_manifest
            .as_ref()
            .filter(|old| old.game_directory != game_directory)
            .filter(|old| validate_game_directory(&old.game_directory).is_ok())
        {
            let old_lovely = old.game_directory.join("version.dll");
            if let Some(backup) = old.lovely_backup.as_ref().filter(|path| path.is_file()) {
                previous_target_rollbacks
                    .push(FileRollback::replace(&old_lovely, &std::fs::read(backup)?)?);
            } else if old.lovely_owned
                && old_lovely.is_file()
                && !other_lovely_mods(&old.mods_directory)
            {
                previous_target_rollbacks.push(FileRollback::remove(&old_lovely)?);
            }
            if old.steam_app_id_owned {
                let old_app_id = old.game_directory.join("steam_appid.txt");
                if let Some(backup) = old
                    .steam_app_id_backup
                    .as_ref()
                    .filter(|path| path.is_file())
                {
                    previous_target_rollbacks
                        .push(FileRollback::replace(&old_app_id, &std::fs::read(backup)?)?);
                } else {
                    previous_target_rollbacks.push(FileRollback::remove(&old_app_id)?);
                }
            }
        }
        self.save_manifest(&manifest)?;
        mod_rollback.commit()?;
        if let Some(rollback) = legacy_mod_rollback.as_mut() {
            rollback.commit();
        }
        runtime_rollback.commit();
        for rollback in &mut previous_runtime_rollbacks {
            rollback.commit();
        }
        if let Some(rollback) = lovely_rollback.as_mut() {
            rollback.commit();
        }
        if let Some(rollback) = app_id_rollback.as_mut() {
            rollback.commit();
        }
        if let Some(rollback) = maintenance_rollback.as_mut() {
            rollback.commit();
        }
        for rollback in &mut previous_target_rollbacks {
            rollback.commit();
        }
        progress(OperationProgress::complete(
            OperationKind::Install,
            "Install / Update completed successfully",
        ));
        Ok(manifest)
    }

    pub fn repair(&self) -> Result<InstallManifest> {
        self.repair_with_progress(|_| {})
    }

    pub fn repair_with_progress<F>(&self, progress: F) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        self.repair_with_progress_platform(true, progress)
    }

    fn repair_with_progress_platform<F>(
        &self,
        apply_platform_changes: bool,
        mut progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        let manifest = self
            .load_manifest()?
            .context("Beatblock Online is not installed")?;
        progress(OperationProgress::step(
            OperationKind::Repair,
            "validation",
            2,
            "Inspecting managed components",
        ));
        let result = self.install_with_progress_platform(
            Some(manifest.game_directory),
            false,
            // Re-detect BeatblockPlus during repair. This automatically heals
            // an adapter mismatch if BeatblockPlus was added or removed after
            // the original BBT installation.
            None,
            manifest.firewall_public,
            apply_platform_changes,
            |mut event| {
                event.operation = OperationKind::Repair;
                if !event.terminal {
                    progress(event);
                }
            },
        )?;
        progress(OperationProgress::complete(
            OperationKind::Repair,
            "Required components were repaired",
        ));
        Ok(result)
    }

    pub fn restore_game_files(&self) -> Result<()> {
        self.restore_with_progress(|_| {})
    }

    pub fn restore_with_progress<F>(&self, mut progress: F) -> Result<()>
    where
        F: FnMut(OperationProgress),
    {
        if beatblock_is_running()
            .context("could not inspect running Beatblock processes before restoring game files")?
        {
            bail!(
                "Beatblock is running. Close Beatblock and refresh the installer before restoring game files"
            );
        }
        progress(OperationProgress::step(
            OperationKind::Restore,
            "validation",
            5,
            "Reading the installation manifest",
        ));
        let manifest = self
            .load_manifest()?
            .context("Beatblock Online is not installed")?;
        validate_game_directory(&manifest.game_directory)
            .context("refusing to restore files outside a valid Beatblock installation")?;
        let mod_directory = manifest.mods_directory.join("BeatblockOnline");
        progress(OperationProgress::step(
            OperationKind::Restore,
            "mod_payload",
            35,
            "Removing the managed mod payload",
        ));
        if mod_directory.is_dir() {
            std::fs::remove_dir_all(mod_directory)?;
        }
        let lovely = manifest.game_directory.join("version.dll");
        progress(OperationProgress::step(
            OperationKind::Restore,
            "lovely",
            70,
            "Restoring the original Lovely injector state",
        ));
        if let Some(backup) = manifest.lovely_backup.as_ref() {
            if backup.is_file() {
                std::fs::copy(backup, lovely)?;
            }
        } else if manifest.lovely_owned && lovely.is_file() {
            std::fs::remove_file(lovely)?;
        }
        let app_id = manifest.game_directory.join("steam_appid.txt");
        if let Some(backup) = manifest.steam_app_id_backup.as_ref() {
            if backup.is_file() {
                std::fs::copy(backup, app_id)?;
            }
        } else if manifest.steam_app_id_owned && app_id.is_file() {
            std::fs::remove_file(app_id)?;
        }
        progress(OperationProgress::complete(
            OperationKind::Restore,
            "Game files were restored",
        ));
        Ok(())
    }

    pub fn set_firewall_profile(&self, public: bool) -> Result<()> {
        let mut manifest = self
            .load_manifest()?
            .context("Beatblock Online is not installed")?;
        let runtime = manifest
            .runtime_path
            .as_deref()
            .context("installed runtime path is missing")?;
        Self::configure_firewall(runtime, public, true)?;
        manifest.firewall_installed = true;
        manifest.firewall_public = public;
        self.save_manifest(&manifest)
    }

    pub fn prepare_renderer_profile(&self) -> Result<PathBuf> {
        let profile = self.data_dir.join("renderer-profile");
        let directory = profile.join("Beatblock/Mods/BeatblockOnlineRenderer");
        std::fs::create_dir_all(directory.join("bbt"))?;
        std::fs::create_dir_all(directory.join("lovely"))?;
        for (relative, bytes) in SHARED_MOD_PAYLOAD {
            let target = directory.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, bytes)?;
        }
        std::fs::write(
            directory.join("lovely/bootstrap.toml"),
            include_bytes!("../../mod/standalone/lovely/bootstrap.toml"),
        )?;
        Ok(profile)
    }

    pub fn install_obs_plugin(&self) -> Result<PathBuf> {
        Ok(self.stage_obs_plugin()?.commit())
    }

    fn stage_obs_plugin(&self) -> Result<ObsInstallTransaction> {
        if OBS_PLUGIN_PAYLOAD.is_empty() {
            bail!("this installer build does not contain the OBS plugin payload");
        }
        validate_obs_payload(OBS_PLUGIN_PAYLOAD)?;
        if self.detected_obs_running().context(
            "could not inspect running OBS Studio processes before installing its source",
        )? {
            bail!("OBS Studio is running. Close OBS before installing or updating the optional OBS source, then retry; administrator access cannot replace a loaded plugin DLL");
        }
        let obs = self
            .obs_directory()
            .context("OBS Studio 32 x64 was not detected; choose its folder in the installer")?;
        let program_data = configured_program_data();
        self.stage_obs_plugin_into(obs, &program_data)
    }

    /// Installs OBS files transactionally into either the selected portable
    /// tree or a supplied ProgramData root. Tests use disposable roots while
    /// exercising the identical layout selection, payload, and marker code.
    #[cfg(test)]
    fn install_obs_plugin_into(&self, obs: PathBuf, program_data: &Path) -> Result<PathBuf> {
        Ok(self.stage_obs_plugin_into(obs, program_data)?.commit())
    }

    fn stage_obs_plugin_into(
        &self,
        obs: PathBuf,
        program_data: &Path,
    ) -> Result<ObsInstallTransaction> {
        // Installed OBS copies use the recommended ProgramData plugin layout.
        // Portable mode deliberately isolates itself from ProgramData, so its
        // marker must route both payloads into OBS' local plugin directories.
        let (plugin, locale) = obs_install_paths(&obs, program_data);
        let locale_payload = include_bytes!("../../obs-plugin/data/locale/en-US.ini");
        let marker = self.data_dir.join("obs-install.json");
        let record = ObsInstallManifest {
            version: env!("CARGO_PKG_VERSION").into(),
            obs_directory: obs,
            plugin: plugin.clone(),
            locale: locale.clone(),
            plugin_sha256: hex::encode(Sha256::digest(OBS_PLUGIN_PAYLOAD)),
        };
        let marker_payload = serde_json::to_vec_pretty(&record)?;
        let plugin_rollback =
            FileRollback::replace(&plugin, OBS_PLUGIN_PAYLOAD).context("install OBS source DLL")?;
        let locale_rollback =
            FileRollback::replace(&locale, locale_payload).context("install OBS source locale")?;
        let marker_rollback = FileRollback::replace(&marker, &marker_payload)
            .context("record OBS source installation")?;
        if !file_matches(&plugin, OBS_PLUGIN_PAYLOAD) || !file_matches(&locale, locale_payload) {
            bail!("OBS source post-install hash verification failed");
        }
        // Alpha releases used the former product name as a separate OBS
        // module. Loading both modules retains duplicate source factories, so
        // remove only those two known installer-owned files transactionally.
        let legacy_root = program_data.join("obs-studio/plugins/beatblock-together-obs");
        let mut legacy_rollbacks = Vec::new();
        for legacy in [
            legacy_root.join("bin/64bit/beatblock-together-obs.dll"),
            legacy_root.join("data/locale/en-US.ini"),
        ] {
            if legacy.is_file() {
                legacy_rollbacks.push(FileRollback::remove(&legacy)?);
            }
        }
        Ok(ObsInstallTransaction {
            plugin,
            plugin_rollback,
            locale_rollback,
            marker_rollback,
            legacy_rollbacks,
        })
    }

    pub fn obs_plugin_available(&self) -> bool {
        self.obs_plugin_payload_available() && self.obs_directory().is_some()
    }

    pub fn obs_plugin_payload_available(&self) -> bool {
        !OBS_PLUGIN_PAYLOAD.is_empty() && validate_obs_payload(OBS_PLUGIN_PAYLOAD).is_ok()
    }

    /// Reports whether OBS currently owns its plugin DLLs so the installer UI
    /// can defer only that optional component instead of blocking core setup.
    pub fn obs_running(&self) -> Result<bool> {
        self.detected_obs_running()
    }

    /// Reports whether any Beatblock process can still own the managed game
    /// files. The GUI and command-line maintenance paths both enforce this.
    pub fn beatblock_running(&self) -> Result<bool> {
        beatblock_is_running()
    }

    /// A recorded OBS installation means uninstall may need to remove a loaded
    /// plugin DLL and therefore must respect the OBS process lock.
    pub fn obs_plugin_managed(&self) -> bool {
        self.data_dir.join("obs-install.json").is_file()
    }

    fn obs_plugin_ready(&self) -> bool {
        let marker = self.data_dir.join("obs-install.json");
        let Ok(bytes) = read_bounded_file(&marker, MAX_OBS_MARKER_BYTES) else {
            return false;
        };
        let Ok(record) = serde_json::from_slice::<ObsInstallManifest>(&bytes) else {
            return false;
        };
        !OBS_PLUGIN_PAYLOAD.is_empty()
            && record.plugin_sha256 == hex::encode(Sha256::digest(OBS_PLUGIN_PAYLOAD))
            && file_matches(&record.plugin, OBS_PLUGIN_PAYLOAD)
            && file_matches(
                &record.locale,
                include_bytes!("../../obs-plugin/data/locale/en-US.ini"),
            )
    }

    /// Starts exactly the selected executable and waits for Lovely's own log
    /// evidence. The game remains running after verification succeeds.
    pub fn launch_and_verify<F>(&self, selected: &Path, mut progress: F) -> Result<LaunchReport>
    where
        F: FnMut(OperationProgress),
    {
        validate_game_directory(selected)?;
        let executable = selected.join("Beatblock.exe");
        progress(OperationProgress::step(
            OperationKind::Launch,
            "preflight",
            10,
            "Preparing the selected game copy",
        ));
        let log_directory = self
            .mods_directory()
            .context("Windows APPDATA is unavailable")?
            .join("lovely/log");
        let app_id_path = selected.join("steam_appid.txt");
        // A test copy may need the Steam application id to start, but an early
        // spawn/log error must not strand this temporary file in the game.
        let _app_id_override = if app_id_path.is_file() {
            None
        } else {
            Some(FileRollback::replace(&app_id_path, b"3045200\n")?)
        };
        let started = std::time::SystemTime::now();
        let mut child = std::process::Command::new(&executable)
            .current_dir(selected)
            .spawn()
            .with_context(|| format!("launch {}", executable.display()))?;
        progress(OperationProgress::step(
            OperationKind::Launch,
            "startup",
            35,
            "Waiting for Lovely initialization",
        ));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut latest_log = None;
        let mut last_text = String::new();
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break Err(anyhow::anyhow!(
                        "Beatblock exited during startup ({status}). {}",
                        lovely_error_excerpt(&last_text)
                    ));
                }
                Ok(None) => {}
                Err(error) => break Err(error).context("inspect Beatblock startup process"),
            }
            if let Some(path) = newest_file_since(&log_directory, started) {
                last_text = std::fs::read_to_string(&path).unwrap_or_default();
                latest_log = Some(path.clone());
                let expected = format!("Game directory is at {:?}", selected);
                if last_text.contains("panicked at") || last_text.contains("ERROR") {
                    break Err(anyhow::anyhow!(
                        "Lovely reported a startup error: {}",
                        lovely_error_excerpt(&last_text)
                    ));
                }
                if last_text.contains("Initialization complete") && last_text.contains(&expected) {
                    progress(OperationProgress::complete(
                        OperationKind::Launch,
                        "Lovely initialized and loaded the selected Beatblock folder",
                    ));
                    break Ok(LaunchReport {
                        executable: executable.clone(),
                        log_path: path,
                        message: "Beatblock is running and all Lovely modules initialized".into(),
                    });
                }
            }
            if Instant::now() >= deadline {
                break Err(anyhow::anyhow!("Beatblock stayed open, but Lovely did not complete initialization within 20 seconds. {}", lovely_error_excerpt(&last_text)));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result.with_context(|| {
            latest_log
                .map(|p| format!("Lovely log: {}", p.display()))
                .unwrap_or_else(|| "Lovely did not create a new log".into())
        })
    }

    pub fn uninstall(&self) -> Result<()> {
        self.uninstall_with_data(false)
    }

    pub fn uninstall_with_data(&self, remove_user_data: bool) -> Result<()> {
        self.uninstall_with_progress(remove_user_data, |_| {})
    }

    pub fn uninstall_with_progress<F>(&self, remove_user_data: bool, progress: F) -> Result<()>
    where
        F: FnMut(OperationProgress),
    {
        self.uninstall_with_progress_platform(remove_user_data, true, progress)
    }

    fn uninstall_with_progress_platform<F>(
        &self,
        remove_user_data: bool,
        apply_platform_changes: bool,
        mut progress: F,
    ) -> Result<()>
    where
        F: FnMut(OperationProgress),
    {
        if beatblock_is_running()
            .context("could not inspect running Beatblock processes before uninstalling")?
        {
            bail!(
                "Beatblock is running. Close Beatblock and refresh the installer before uninstalling"
            );
        }
        if self.obs_plugin_managed()
            && self
                .detected_obs_running()
                .context("could not inspect running OBS Studio processes before uninstalling")?
        {
            bail!(
                "OBS Studio is running. Close OBS and refresh the installer before removing its managed plugin"
            );
        }
        progress(OperationProgress::step(
            OperationKind::Uninstall,
            "validation",
            5,
            "Reading the managed installation",
        ));
        let Some(manifest) = self.load_manifest()? else {
            progress(OperationProgress::complete(
                OperationKind::Uninstall,
                "Beatblock Online is already uninstalled",
            ));
            return Ok(());
        };
        let mod_directory = manifest.mods_directory.join("BeatblockOnline");
        if mod_directory.exists() {
            progress(OperationProgress::step(
                OperationKind::Uninstall,
                "mod_payload",
                25,
                "Removing the in-game adapter and shared payload",
            ));
            std::fs::remove_dir_all(mod_directory)?;
        }
        // A stale game directory must not turn an elevated uninstall into a
        // general-purpose file deletion primitive. Other product-owned state
        // can still be removed when the game itself has already disappeared.
        if validate_game_directory(&manifest.game_directory).is_ok() {
            let lovely = manifest.game_directory.join("version.dll");
            if let Some(backup) = manifest.lovely_backup.as_ref() {
                progress(OperationProgress::step(
                    OperationKind::Uninstall,
                    "lovely",
                    45,
                    "Restoring the preserved injector",
                ));
                if backup.is_file() {
                    std::fs::copy(backup, lovely)?;
                }
            } else if manifest.lovely_owned
                && lovely.is_file()
                && !other_lovely_mods(&manifest.mods_directory)
            {
                std::fs::remove_file(lovely)?;
            }
            let app_id = manifest.game_directory.join("steam_appid.txt");
            if let Some(backup) = manifest.steam_app_id_backup.as_ref() {
                if backup.is_file() {
                    std::fs::copy(backup, app_id)?;
                }
            } else if manifest.steam_app_id_owned && app_id.is_file() {
                std::fs::remove_file(app_id)?;
            }
        }
        let manifest_path = self.manifest_path();
        if manifest_path.is_file() {
            std::fs::remove_file(manifest_path)?;
        }
        if let Some(runtime) = manifest.runtime_path.as_ref() {
            progress(OperationProgress::step(
                OperationKind::Uninstall,
                "system_changes",
                65,
                "Removing runtime and firewall registration",
            ));
            if apply_platform_changes {
                let _ = Self::configure_firewall(runtime, false, false);
            }
            let _ = std::fs::remove_file(runtime);
        }
        // Also clean a stale alpha runtime if a hand-edited or partial manifest
        // no longer references it directly.
        let legacy_runtime = self.data_dir.join("runtime").join(LEGACY_RUNTIME_FILE_NAME);
        let _ = std::fs::remove_file(legacy_runtime);
        if apply_platform_changes {
            let _ = self.unregister_uninstall();
        }
        if let Ok(bytes) = read_bounded_file(
            &self.data_dir.join("obs-install.json"),
            MAX_OBS_MARKER_BYTES,
        ) {
            if let Ok(record) = serde_json::from_slice::<ObsInstallManifest>(&bytes) {
                if obs_record_paths_are_managed(&record) {
                    let _ = std::fs::remove_file(record.plugin);
                    let _ = std::fs::remove_file(record.locale);
                }
            } else if let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(&bytes) {
                // Alpha manifests used a two-path array in the legacy OBS
                // layout. Only retain cleanup for the two known filenames
                // beneath an OBS plugin directory.
                for path in paths
                    .into_iter()
                    .filter(|path| legacy_obs_path_is_managed(path))
                {
                    let _ = std::fs::remove_file(path);
                }
            }
            let _ = std::fs::remove_file(self.data_dir.join("obs-install.json"));
        }
        if let Some(installer) = manifest.maintenance_installer {
            if installer != std::env::current_exe().unwrap_or_default() {
                let _ = std::fs::remove_file(installer);
            }
        }
        if remove_user_data {
            progress(OperationProgress::step(
                OperationKind::Uninstall,
                "user_data",
                85,
                "Removing settings and match history",
            ));
            for path in [
                "runtime.sqlite3",
                "manager.sqlite3",
                "journals",
                "exports",
                "local-token.txt",
                "config.json",
            ] {
                let path = self.data_dir.join(path);
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(path);
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        progress(OperationProgress::complete(
            OperationKind::Uninstall,
            "Uninstall completed successfully",
        ));
        Ok(())
    }

    /// Program-scoped UDP access lets the host choose a port in-game without
    /// requesting elevation for each room.
    pub fn configure_firewall(runtime: &Path, public: bool, add: bool) -> Result<()> {
        #[cfg(windows)]
        {
            // `add rule` is not idempotent. Always remove an older BBT rule
            // first and deliberately tolerate an absent rule. This also makes
            // profile changes a single, predictable operation.
            if add {
                let mut remove = firewall_command(runtime, public, false);
                let _ = hidden(&mut remove);
            }
            let mut command = firewall_command(runtime, public, add);
            let output = hidden(&mut command).context("start Windows Firewall configuration")?;
            if !output.status.success() {
                let details = command_output_details(&output);
                bail!(
                    "Windows Firewall {} failed (exit {}): {}",
                    if add { "rule creation" } else { "rule removal" },
                    output.status.code().unwrap_or(-1),
                    details
                );
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn request_elevated(arguments: &str) -> Result<()> {
        let status =
            std::env::temp_dir().join(format!("bbt-elevated-{}.json", uuid::Uuid::new_v4()));
        Self::request_elevated_with_progress(arguments, &status, |_| {})
    }

    #[cfg(windows)]
    pub fn request_elevated_with_progress<F>(
        arguments: &str,
        status_path: &Path,
        mut progress: F,
    ) -> Result<()>
    where
        F: FnMut(OperationProgress),
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_FAILED, WAIT_OBJECT_0},
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
            UI::{
                Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
                WindowsAndMessaging::SW_HIDE,
            },
        };
        let operation = wide("runas");
        let executable = wide(std::env::current_exe()?.as_os_str());
        let combined = format!(
            "{arguments} --operation-file {}",
            quote_windows(status_path)
        );
        let arguments = wide(combined);
        // Populate the Win32 request atomically so every pointer remains tied to
        // the owned UTF-16 buffers above for the duration of ShellExecuteExW.
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: operation.as_ptr(),
            lpFile: executable.as_ptr(),
            lpParameters: arguments.as_ptr(),
            nShow: SW_HIDE,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
            let error = std::io::Error::last_os_error();
            let _ = std::fs::remove_file(status_path);
            bail!("administrator approval was cancelled or Windows could not start the helper: {error}");
        }
        let mut last = None;
        let mut last_event = None;
        loop {
            if let Some(event) = read_operation_progress(status_path) {
                if last != Some((event.percent, event.phase.clone())) {
                    last = Some((event.percent, event.phase.clone()));
                    last_event = Some(event.clone());
                    progress(event);
                }
            }
            let wait = unsafe { WaitForSingleObject(info.hProcess, 200) };
            if wait == WAIT_OBJECT_0 {
                break;
            }
            if wait == WAIT_FAILED {
                let error = std::io::Error::last_os_error();
                unsafe { CloseHandle(info.hProcess) };
                let _ = std::fs::remove_file(status_path);
                bail!("waiting for the administrator helper failed: {error}");
            }
        }
        let mut code = 1u32;
        let read_exit_code = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
        let exit_code_error = (read_exit_code == 0).then(std::io::Error::last_os_error);
        unsafe {
            CloseHandle(info.hProcess);
        }
        // The helper can write its terminal status and exit between the final
        // polling read and WaitForSingleObject. Always perform one post-exit
        // read so the UI reports the real failure instead of only exit code 1.
        if let Some(event) = read_operation_progress(status_path) {
            if last != Some((event.percent, event.phase.clone())) {
                last_event = Some(event.clone());
                progress(event);
            }
        }
        if let Some(error) = exit_code_error {
            bail!(
                "reading the administrator helper result failed: {error}; helper status: {}",
                status_path.display()
            );
        }
        if code != 0 {
            let detail = last_event
                .as_ref()
                .filter(|event| event.terminal && event.severity == Severity::Error)
                .map(|event| event.message.as_str());
            if let Some(detail) = detail {
                bail!(
                    "{detail} (administrator helper status: {})",
                    status_path.display()
                );
            }
            bail!(
                "administrator helper failed with exit code {code} and did not publish a terminal error; helper status: {}",
                status_path.display()
            );
        }
        let _ = std::fs::remove_file(status_path);
        Ok(())
    }

    fn register_uninstall(&self, manifest: &InstallManifest) -> Result<()> {
        #[cfg(windows)]
        {
            let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\BeatblockOnline";
            let current = std::env::current_exe()?;
            let exe = manifest
                .maintenance_installer
                .as_ref()
                .unwrap_or(&current)
                .display()
                .to_string();
            for (name, value) in [
                ("DisplayName", "Beatblock Online".to_string()),
                ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
                ("Publisher", "Beatblock Online".to_string()),
                ("DisplayIcon", exe.clone()),
                ("UninstallString", format!("\"{exe}\" --uninstall-now")),
            ] {
                let mut command = std::process::Command::new("reg.exe");
                command.args(["add", key, "/v", name, "/t", "REG_SZ", "/d", &value, "/f"]);
                if !hidden(&mut command)?.status.success() {
                    bail!("could not register installed application");
                }
            }
        }
        Ok(())
    }

    fn unregister_uninstall(&self) -> Result<()> {
        #[cfg(windows)]
        {
            let mut command = std::process::Command::new("reg.exe");
            command.args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\BeatblockOnline",
                "/f",
            ]);
            let _ = hidden(&mut command)?;
        }
        Ok(())
    }

    fn find_game_directory(&self) -> Option<PathBuf> {
        if let Ok(path) = std::env::var("BBT_GAME_DIR") {
            let path = PathBuf::from(path);
            if validate_game_directory(&path).is_ok() {
                return Some(path);
            }
        }
        steam_game_directories()
            .into_iter()
            .find(|path| validate_game_directory(path).is_ok())
    }

    fn is_steam_managed_game_directory(&self, selected: &Path) -> bool {
        steam_game_directories()
            .iter()
            .any(|candidate| paths_equal(candidate, selected))
    }

    fn manifest_path(&self) -> PathBuf {
        self.data_dir.join("install-manifest.json")
    }

    /// Treat the on-disk manifest as untrusted input. Maintenance operations
    /// may run elevated, so every persisted path must still resolve to one of
    /// the small set of locations this installer owns.
    fn validate_manifest_paths(&self, manifest: &InstallManifest) -> Result<()> {
        let expected_mods = self
            .mods_directory()
            .unwrap_or_else(|| self.data_dir.join("Mods"));
        if !paths_equal(&manifest.mods_directory, &expected_mods) {
            bail!("installation manifest names an unmanaged Mods directory");
        }
        if manifest.game_directory.file_name().is_none() {
            bail!("installation manifest names an unsafe game directory");
        }

        let backup_root = self.data_dir.join("backups");
        if let Some(path) = manifest.lovely_backup.as_ref() {
            let expected = backup_root.join(backup_name_for(&manifest.game_directory));
            if !paths_equal(path, &expected) {
                bail!("installation manifest names an unmanaged Lovely backup");
            }
        }
        if let Some(path) = manifest.steam_app_id_backup.as_ref() {
            let expected = backup_root.join(app_id_backup_name_for(&manifest.game_directory));
            if !paths_equal(path, &expected) {
                bail!("installation manifest names an unmanaged Steam app-id backup");
            }
        }

        if let Some(path) = manifest.runtime_path.as_ref() {
            let runtime_root = self.data_dir.join("runtime");
            let current = runtime_root.join(RUNTIME_FILE_NAME);
            let legacy = runtime_root.join(LEGACY_RUNTIME_FILE_NAME);
            if !paths_equal(path, &current) && !paths_equal(path, &legacy) {
                bail!("installation manifest names an unmanaged runtime");
            }
        }
        if let Some(path) = manifest.maintenance_installer.as_ref() {
            let expected = self
                .data_dir
                .join("installer")
                .join("BeatblockOnlineInstaller.exe");
            if !paths_equal(path, &expected) {
                bail!("installation manifest names an unmanaged maintenance installer");
            }
        }

        for relative in manifest.file_hashes.keys() {
            if !is_clean_relative_path(relative) {
                bail!("installation manifest contains an unsafe managed-file path");
            }
        }
        Ok(())
    }

    fn load_manifest(&self) -> Result<Option<InstallManifest>> {
        let path = self.manifest_path();
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = read_bounded_file(&path, MAX_INSTALL_MANIFEST_BYTES)?;
        // Windows maintenance tools sometimes rewrite JSON with a UTF-8 BOM.
        // Accept it as a legacy migration input; all new writes remain BOM-free.
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        let manifest: InstallManifest = serde_json::from_slice(bytes)?;
        self.validate_manifest_paths(&manifest)?;
        Ok(Some(manifest))
    }

    fn save_manifest(&self, manifest: &InstallManifest) -> Result<()> {
        self.validate_manifest_paths(manifest)?;
        std::fs::create_dir_all(&self.data_dir)?;
        atomic_write(&self.manifest_path(), &serde_json::to_vec_pretty(manifest)?)?;
        Ok(())
    }
}

fn steam_game_directories() -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles(x86)") {
        libraries.push(PathBuf::from(program_files).join("Steam"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        libraries.push(PathBuf::from(program_files).join("Steam"));
    }
    let mut roots = libraries.clone();
    for steam in libraries {
        let vdf = steam.join("steamapps/libraryfolders.vdf");
        if let Ok(content) = std::fs::read_to_string(vdf) {
            for line in content.lines() {
                if !line.contains("\"path\"") {
                    continue;
                }
                let values = line.split('"').collect::<Vec<_>>();
                if values.len() >= 4 {
                    roots.push(PathBuf::from(values[3].replace("\\\\", "\\")));
                }
            }
        }
    }
    roots
        .into_iter()
        .map(|root| root.join("steamapps/common/Beatblock"))
        .collect()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    fn normalized(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_owned())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['\\', '/'])
            .replace('/', r"\")
            .to_ascii_lowercase()
    }
    normalized(left) == normalized(right)
}

fn is_clean_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn read_bounded_file(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > maximum_bytes {
        bail!(
            "{} exceeds the {} byte safety limit",
            path.display(),
            maximum_bytes
        );
    }
    Ok(std::fs::read(path)?)
}

fn component(
    name: &str,
    state: ComponentState,
    label: &str,
    included: &str,
    details: impl Into<String>,
) -> ComponentStatus {
    ComponentStatus {
        name: name.into(),
        state,
        label: label.into(),
        included: included.into(),
        details: details.into(),
    }
}

fn state_for(installed: bool, healthy: bool) -> ComponentState {
    if healthy {
        ComponentState::Ready
    } else if installed {
        ComponentState::Broken
    } else {
        ComponentState::NotInstalled
    }
}

pub fn distribution_label(value: Distribution) -> &'static str {
    match value {
        Distribution::Standalone => "Standalone Lovely",
        Distribution::BeatblockPlus => "BeatblockPlus 2.x",
    }
}

fn file_matches(path: &Path, expected: &[u8]) -> bool {
    std::fs::read(path)
        .ok()
        .is_some_and(|bytes| Sha256::digest(&bytes)[..] == Sha256::digest(expected)[..])
}

/// Avoid replacing a working injector just because it was built or signed
/// differently. The PE header and embedded crate identity distinguish Lovely
/// from an arbitrary DLL while keeping compatibility independent of one hash.
fn is_lovely_injector(path: &Path) -> bool {
    const MAX_INJECTOR_SIZE: u64 = 64 * 1024 * 1024;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 2 || metadata.len() > MAX_INJECTOR_SIZE {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes.starts_with(b"MZ")
        && bytes
            .windows(b"lovely-injector".len())
            .any(|window| window == b"lovely-injector")
}

fn validate_staged_payload(directory: &Path, distribution: Distribution) -> Result<()> {
    for (relative, bytes) in SHARED_MOD_PAYLOAD {
        if !file_matches(&directory.join(relative), bytes) {
            bail!("staged payload failed verification: {relative}");
        }
    }
    let adapter = match distribution {
        Distribution::Standalone => vec!["lovely/bootstrap.toml"],
        Distribution::BeatblockPlus => {
            vec!["mod.json", "main.lua", "config.lua", "states/Online.lua"]
        }
    };
    for relative in adapter {
        if !directory.join(relative).is_file() {
            bail!("staged adapter is missing {relative}");
        }
    }
    validate_online_recovery_contract(directory)?;
    Ok(())
}

/// The installer is the public distribution boundary, so verify the staged
/// Lua has the recovery behavior that prevents host/join actions hanging after
/// an IPC loss. Hash checks catch corruption; these markers catch stale builds.
fn validate_online_recovery_contract(directory: &Path) -> Result<()> {
    let contracts = [
        ("bbt/core.lua", "pendingRequestDeadlineMs"),
        ("bbt/core.lua", "message.type == 'runtime.disconnected'"),
        ("bbt/core.lua", "CLIENT_INSTANCE_ID"),
        ("bbt/core.lua", "BBT.send('client.ping'"),
        ("bbt/core.lua", "message.type == 'runtime.heartbeat'"),
        ("bbt/ipc_thread.lua", "runtime.disconnected"),
        ("bbt/ipc_thread.lua", "launchAttempts"),
        ("bbt/ipc_thread.lua", "CreateProcessA"),
        ("lovely/hooks.toml", "loc.json.bbtOnline"),
        ("bbt/core.lua", "protocolVersion = 3"),
        ("bbt/online_state.lua", "local function bounded"),
        ("bbt/online_state.lua", "HOST ADDRESS"),
        ("bbt/online_state.lua", "room.commentator_set"),
        ("bbt/online_state.lua", "broadcast.mirror_set"),
        ("bbt/dashboard_components.lua", "font:getHeight()"),
        ("bbt/dashboard_components.lua", "height < 22"),
        ("bbt/renderer.lua", "readbackPending = {false,false}"),
        ("bbt/renderer.lua", "dpiscale=1"),
        ("bbt/renderer.lua", "Renderer.frames.pointer + 32"),
        ("bbt/renderer.lua", "capturePlayerView"),
        ("bbt/renderer.lua", "function Renderer.applyClock()"),
        ("bbt/renderer.lua", "function Renderer.steerPaddle()"),
    ];
    for (relative, marker) in contracts {
        let source = std::fs::read_to_string(directory.join(relative))
            .with_context(|| format!("read staged recovery module {relative}"))?;
        if !source.contains(marker) {
            bail!("staged payload is missing Online recovery contract in {relative}: {marker}");
        }
    }
    Ok(())
}

fn collect_managed_hashes(directory: &Path) -> Result<std::collections::BTreeMap<PathBuf, String>> {
    let mut hashes = std::collections::BTreeMap::new();
    if !directory.is_dir() {
        return Ok(hashes);
    }
    for entry in walkdir::WalkDir::new(directory)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let relative = entry.path().strip_prefix(directory)?.to_owned();
        hashes.insert(relative, sha256_file(entry.path())?);
    }
    Ok(hashes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes)?;
    let replace = crate::exports::replace_file(&temporary, path);
    if replace.is_err() {
        // Failed replacements must not strand payload bytes beside the target.
        // This is especially important when a damaged install has a directory
        // where a managed file should be.
        let _ = std::fs::remove_file(&temporary);
    }
    replace?;
    Ok(())
}

/// Restores the prior file bytes if a later phase of the install fails.
struct FileRollback {
    path: PathBuf,
    previous: Option<Vec<u8>>,
    committed: bool,
}
impl FileRollback {
    fn replace(path: &Path, bytes: &[u8]) -> Result<Self> {
        let previous = std::fs::read(path).ok();
        atomic_write(path, bytes)?;
        Ok(Self {
            path: path.to_owned(),
            previous,
            committed: false,
        })
    }
    fn remove(path: &Path) -> Result<Self> {
        let previous = std::fs::read(path).ok();
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        Ok(Self {
            path: path.to_owned(),
            previous,
            committed: false,
        })
    }
    fn commit(&mut self) {
        self.committed = true;
    }
}

fn backup_name_for(game_directory: &Path) -> String {
    let hash = hex::encode(Sha256::digest(game_directory.to_string_lossy().as_bytes()));
    format!("version-{}.before-bbt.dll", &hash[..12])
}

fn app_id_backup_name_for(game_directory: &Path) -> String {
    let hash = hex::encode(Sha256::digest(game_directory.to_string_lossy().as_bytes()));
    format!("steam-appid-{}.before-bbt.txt", &hash[..12])
}
impl Drop for FileRollback {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Some(bytes) = self.previous.as_deref() {
            let _ = atomic_write(&self.path, bytes);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Owns an activated staged directory until every non-directory component has
/// passed postflight. Drop provides automatic rollback on every `?` path.
struct DirectoryRollback {
    target: PathBuf,
    backup: PathBuf,
    committed: bool,
}
impl DirectoryRollback {
    fn activate(stage: &Path, target: &Path, backup: &Path) -> Result<Self> {
        if target.exists() {
            std::fs::rename(target, backup).context("move previous mod into rollback storage")?;
        }
        if let Err(error) = std::fs::rename(stage, target) {
            if backup.exists() {
                let _ = std::fs::rename(backup, target);
            }
            return Err(error).context("activate staged Beatblock Online mod");
        }
        Ok(Self {
            target: target.to_owned(),
            backup: backup.to_owned(),
            committed: false,
        })
    }
    fn commit(&mut self) -> Result<()> {
        if self.backup.exists() {
            std::fs::remove_dir_all(&self.backup)?;
        }
        self.committed = true;
        Ok(())
    }
}
impl Drop for DirectoryRollback {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.target);
        if self.backup.exists() {
            let _ = std::fs::rename(&self.backup, &self.target);
        }
    }
}

/// Temporarily removes an installer-owned legacy mod from Lovely's scan path.
/// The old directory is restored automatically unless the replacement install
/// reaches its durable commit point.
struct DirectoryRemovalRollback {
    target: PathBuf,
    backup: PathBuf,
    committed: bool,
}
impl DirectoryRemovalRollback {
    fn quarantine(target: &Path, backup: &Path) -> Result<Self> {
        std::fs::rename(target, backup).context("quarantine the retired BeatblockTogether mod")?;
        Ok(Self {
            target: target.to_owned(),
            backup: backup.to_owned(),
            committed: false,
        })
    }

    fn commit(&mut self) {
        // Once the current manifest and mod are durable, never put the retired
        // hook back into Lovely's scan path. A locked backup is harmless and a
        // later installer run can remove it.
        self.committed = true;
        let _ = std::fs::remove_dir_all(&self.backup);
    }
}
impl Drop for DirectoryRemovalRollback {
    fn drop(&mut self) {
        if !self.committed && self.backup.exists() {
            let _ = std::fs::rename(&self.backup, &self.target);
        }
    }
}

fn is_managed_legacy_mod(directory: &Path) -> bool {
    let readme = std::fs::read_to_string(directory.join("README.txt")).unwrap_or_default();
    readme.contains(
        "Beatblock Together standalone Lovely package. Installed by BeatblockTogetherInstaller.exe.",
    ) && directory.join("bbt/core.lua").is_file()
        && directory.join("lovely/bootstrap.toml").is_file()
}

fn newest_file_since(directory: &Path, started: std::time::SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file() && metadata.modified().ok()? >= started)
                .then_some((metadata.modified().ok()?, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

fn lovely_error_excerpt(text: &str) -> String {
    let lines = text
        .lines()
        .rev()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("error") || lower.contains("panic") || lower.contains("not found")
        })
        .take(3)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "No Lovely error was logged.".into()
    } else {
        lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
    }
}

pub fn write_operation_status(path: &Path, event: &OperationProgress) -> Result<()> {
    atomic_write(path, &serde_json::to_vec(event)?)
}

fn read_operation_progress(path: &Path) -> Option<OperationProgress> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(windows)]
fn quote_windows(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(windows)]
fn obs_is_running() -> Result<bool> {
    process_is_running("obs64.exe")
}

#[cfg(windows)]
fn beatblock_is_running() -> Result<bool> {
    process_is_running("Beatblock.exe")
}

#[cfg(windows)]
fn process_is_running(executable_name: &str) -> Result<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        let error = unsafe { GetLastError() };
        bail!("could not create a process snapshot (Windows error {error})");
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        let error = unsafe { GetLastError() };
        unsafe { CloseHandle(snapshot) };
        bail!("could not start process enumeration (Windows error {error})");
    }
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case(executable_name) {
            unsafe { CloseHandle(snapshot) };
            return Ok(true);
        }
        if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(snapshot) };
            if error == ERROR_NO_MORE_FILES {
                return Ok(false);
            }
            bail!("process enumeration failed (Windows error {error})");
        }
    }
}

#[cfg(not(windows))]
fn obs_is_running() -> Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
fn beatblock_is_running() -> Result<bool> {
    Ok(false)
}

fn detect_obs_directory() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os("BBT_OBS_DIR") {
        candidates.push(PathBuf::from(explicit));
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("obs-studio"));
        }
    }
    candidates
        .into_iter()
        .find_map(|candidate| normalize_obs_directory(&candidate))
}

/// Resolves selections of the OBS root, `bin`, `bin/64bit`, or `obs64.exe`
/// back to the root. Supporting the nearby forms makes typed and portable
/// locations forgiving without recursively scanning the user's drives.
fn normalize_obs_directory(candidate: &Path) -> Option<PathBuf> {
    candidate.ancestors().take(4).find_map(|root| {
        if root.join("bin/64bit/obs64.exe").is_file() {
            // Preserve the user-facing spelling. Windows canonicalization adds
            // a `\\?\` prefix that is valid internally but confusing in the UI.
            Some(root.to_path_buf())
        } else {
            None
        }
    })
}

fn obs_program_data_paths(program_data: &Path) -> (PathBuf, PathBuf) {
    let root = program_data.join("obs-studio/plugins/beatblock-online-obs");
    (
        root.join("bin/64bit/beatblock-online-obs.dll"),
        root.join("data/locale/en-US.ini"),
    )
}

fn obs_portable_paths(obs_directory: &Path) -> (PathBuf, PathBuf) {
    (
        obs_directory.join("obs-plugins/64bit/beatblock-online-obs.dll"),
        obs_directory.join("data/obs-plugins/beatblock-online-obs/locale/en-US.ini"),
    )
}

fn obs_directory_is_portable(obs_directory: &Path) -> bool {
    // OBS accepts both spellings; the official Windows ZIP workflow commonly
    // uses `portable_mode.txt`. Avoid treating every custom path as portable,
    // because a normally configured unpacked copy still loads ProgramData.
    obs_directory.join("portable_mode.txt").is_file()
        || obs_directory.join("portable_mode").is_file()
}

fn obs_install_paths(obs_directory: &Path, program_data: &Path) -> (PathBuf, PathBuf) {
    if obs_directory_is_portable(obs_directory) {
        obs_portable_paths(obs_directory)
    } else {
        obs_program_data_paths(program_data)
    }
}

fn configured_program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

fn obs_record_paths_are_managed(record: &ObsInstallManifest) -> bool {
    let (plugin, locale) = obs_program_data_paths(&configured_program_data());
    if paths_equal(&record.plugin, &plugin) && paths_equal(&record.locale, &locale) {
        return true;
    }
    // Portable cleanup is allowed only while the recorded root still validates
    // as the same portable OBS installation. This prevents a forged elevated
    // marker from turning uninstall into an arbitrary two-file deletion.
    let Some(obs_directory) = normalize_obs_directory(&record.obs_directory) else {
        return false;
    };
    if !obs_directory_is_portable(&obs_directory) {
        return false;
    }
    let (plugin, locale) = obs_portable_paths(&obs_directory);
    paths_equal(&record.plugin, &plugin) && paths_equal(&record.locale, &locale)
}

fn legacy_obs_path_is_managed(path: &Path) -> bool {
    let root = configured_program_data().join("obs-studio/plugins/beatblock-together-obs");
    [
        root.join("bin/64bit/beatblock-together-obs.dll"),
        root.join("data/locale/en-US.ini"),
    ]
    .iter()
    .any(|expected| paths_equal(path, expected))
}

fn validate_obs_payload(payload: &[u8]) -> Result<()> {
    if payload.len() < 4096 || !payload.starts_with(b"MZ") {
        bail!("the bundled OBS source is not a valid Windows DLL");
    }
    for export in [
        b"obs_module_load".as_slice(),
        b"obs_module_ver".as_slice(),
        b"obs_module_set_pointer".as_slice(),
    ] {
        if !payload
            .windows(export.len())
            .any(|candidate| candidate == export)
        {
            bail!(
                "the bundled OBS source is missing required export {}",
                String::from_utf8_lossy(export)
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn hidden(command: &mut std::process::Command) -> Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    Ok(command
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()?)
}

#[cfg(windows)]
fn firewall_command(runtime: &Path, public: bool, add: bool) -> std::process::Command {
    let mut command = std::process::Command::new("netsh.exe");
    command.args([
        "advfirewall",
        "firewall",
        if add { "add" } else { "delete" },
        "rule",
        "name=Beatblock Online Host",
    ]);
    if add {
        // PathBuf preserves forward slashes that appear in a joined string.
        // Win32 file APIs accept those paths, but netsh validates `program=`
        // strictly and rejects them as invalid application paths.
        let program = format!("program={}", runtime.to_string_lossy().replace('/', "\\"));
        command.args([
            "dir=in",
            "action=allow",
            "protocol=UDP",
            &program,
            if public {
                "profile=private,public"
            } else {
                "profile=private,domain"
            },
        ]);
    }
    command
}

#[cfg(windows)]
fn command_output_details(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout} | {stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "Windows returned no diagnostic text".into(),
    }
}

#[cfg(windows)]
fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn write(path: &Path, bytes: &[u8], files: &mut Vec<PathBuf>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    files.push(path.to_owned());
    Ok(())
}

fn validate_game_directory(path: &Path) -> Result<()> {
    for file in ["Beatblock.exe", "love.dll", "lua51.dll"] {
        if !path.join(file).is_file() {
            bail!("{} does not contain {file}", path.display());
        }
    }
    let packed = path.join("packed");
    for archive in ["data.zip", "obj.zip", "states.zip"] {
        if !packed.join(archive).is_file() {
            bail!("{} is missing packed/{archive}", path.display());
        }
    }
    Ok(())
}

fn default_mods_directory() -> Option<PathBuf> {
    // Lovely's override is the safe injection path for disposable test copies
    // and isolated renderer profiles; normal installs fall through to APPDATA.
    std::env::var_os("LOVELY_MOD_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("Beatblock/Mods"))
        })
        .or_else(|| {
            ProjectDirs::from("org", "BeatblockOnline", "BeatblockOnline")
                .map(|dirs| dirs.data_dir().join("Mods"))
        })
}

fn find_beatblock_plus(mods: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(mods) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let manifest = entry.path().join("mod.json");
        std::fs::read_to_string(manifest)
            .ok()
            .is_some_and(|value| value.contains("beatblock-plus"))
    })
}

fn other_lovely_mods(mods: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(mods) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Active staging/rollback folders are part of BBT's own atomic
        // transaction and must not make an old injector look externally owned
        // while moving the managed installation to another game folder.
        name != "BeatblockOnline"
            && !name.starts_with(".BeatblockOnline.stage-")
            && !name.starts_with(".BeatblockOnline.rollback-")
            && entry.path().join("lovely").is_dir()
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_game(root: &Path) {
        std::fs::create_dir_all(root.join("packed")).unwrap();
        for file in ["Beatblock.exe", "love.dll", "lua51.dll"] {
            std::fs::write(root.join(file), b"fixture").unwrap();
        }
        for file in ["data.zip", "obj.zip", "states.zip"] {
            std::fs::write(root.join("packed").join(file), b"zip").unwrap();
        }
    }

    fn fake_managed_legacy_mod(mods: &Path) -> PathBuf {
        let legacy = mods.join("BeatblockTogether");
        std::fs::create_dir_all(legacy.join("bbt")).unwrap();
        std::fs::create_dir_all(legacy.join("lovely")).unwrap();
        std::fs::write(
            legacy.join("README.txt"),
            b"Beatblock Together standalone Lovely package. Installed by BeatblockTogetherInstaller.exe.\n",
        )
        .unwrap();
        std::fs::write(legacy.join("bbt/core.lua"), b"legacy runtime bootstrap").unwrap();
        std::fs::write(legacy.join("lovely/bootstrap.toml"), b"[manifest]\n").unwrap();
        legacy
    }

    fn install_isolated(
        installer: &Installer,
        game: &Path,
        distribution: Option<Distribution>,
        progress: &mut Vec<OperationProgress>,
    ) -> Result<InstallManifest> {
        installer.install_with_progress_platform(
            Some(game.to_owned()),
            true,
            distribution,
            false,
            false,
            |event| progress.push(event),
        )
    }

    #[test]
    fn detector_accepts_isolated_test_game_shape() {
        let explicit_fixture = std::env::var_os("BBT_GAME_FIXTURE").map(PathBuf::from);
        let root = explicit_fixture.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!("bbt-isolated-game-shape-{}", rand::random::<u64>()))
        });
        // A plain `cargo test` must not depend on a developer's ignored .test
        // directory. An explicitly supplied physical fixture is still useful,
        // while the default path now exercises the same shape in isolation.
        if explicit_fixture.is_none() {
            fake_game(&root);
        }
        assert!(validate_game_directory(&root).is_ok());
        let installer = Installer::with_mods_directory(
            root.join(".bbt-test-data"),
            root.join(".bbt-test-mods"),
        );
        let inspection = installer.inspect_target(&root);
        assert!(inspection.compatible_layout);
        assert_eq!(inspection.components[0].label, "Compatible");
        if explicit_fixture.is_none() {
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn adapter_detection_is_exclusive_and_requires_a_plus_manifest() {
        let root = std::env::temp_dir().join(format!("bbt-adapters-{}", rand::random::<u64>()));
        std::fs::create_dir_all(root.join("Cosmetic/lovely")).unwrap();
        assert!(!find_beatblock_plus(&root));
        std::fs::create_dir_all(root.join("BeatblockPlus")).unwrap();
        std::fs::write(
            root.join("BeatblockPlus/mod.json"),
            r#"{"id":"beatblock-plus"}"#,
        )
        .unwrap();
        assert!(find_beatblock_plus(&root));
        assert!(other_lovely_mods(&root));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn arbitrary_unicode_game_folder_is_valid_without_a_build_allowlist() {
        let root = std::env::temp_dir().join(format!("BBT Player 测试 {}", rand::random::<u64>()));
        let game = root.join("My Beatblock Copy");
        fake_game(&game);
        let installer = Installer::with_mods_directory(root.join("data"), root.join("mods"));
        let inspection = installer.inspect_target(&game);
        assert!(inspection.valid);
        assert!(inspection.compatible_layout);
        assert_eq!(inspection.install_state, "NOT INSTALLED");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_dashboard_module_is_reported_as_repair_required() {
        let root =
            std::env::temp_dir().join(format!("bbt-missing-dashboard-{}", rand::random::<u64>()));
        let game = root.join("game");
        let mods = root.join("mods");
        let data = root.join("data");
        fake_game(&game);
        let mod_dir = mods.join("BeatblockOnline");
        std::fs::create_dir_all(mod_dir.join("bbt")).unwrap();
        std::fs::create_dir_all(mod_dir.join("lovely")).unwrap();
        for (relative, bytes) in SHARED_MOD_PAYLOAD {
            if *relative != "bbt/dashboard_model.lua" {
                let path = mod_dir.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        std::fs::write(
            mod_dir.join("lovely/bootstrap.toml"),
            include_bytes!("../../mod/standalone/lovely/bootstrap.toml"),
        )
        .unwrap();
        std::fs::write(game.join("version.dll"), LOVELY_PAYLOAD).unwrap();
        let runtime = data.join("runtime").join(RUNTIME_FILE_NAME);
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&runtime, RUNTIME_PAYLOAD).unwrap();
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());
        installer
            .save_manifest(&InstallManifest {
                version: "legacy".into(),
                game_directory: game.clone(),
                mods_directory: mods,
                distribution: Distribution::Standalone,
                installed_files: vec![],
                lovely_owned: true,
                lovely_backup: None,
                steam_app_id_owned: false,
                steam_app_id_backup: None,
                runtime_path: Some(runtime),
                maintenance_installer: None,
                firewall_installed: false,
                firewall_public: false,
                file_hashes: Default::default(),
                lovely_original_sha256: None,
            })
            .unwrap();
        let inspection = installer.inspect_target(&game);
        assert!(inspection.repair_required);
        assert_eq!(inspection.install_state, "REPAIR REQUIRED");
        assert!(inspection
            .components
            .iter()
            .any(|c| c.name == "Shared Lua payload" && c.state == ComponentState::Broken));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn selecting_another_valid_folder_requires_a_move() {
        let root = std::env::temp_dir().join(format!("bbt-move-target-{}", rand::random::<u64>()));
        let old = root.join("old game");
        let new = root.join("new game");
        fake_game(&old);
        fake_game(&new);
        let installer = Installer::with_mods_directory(root.join("data"), root.join("mods"));
        installer
            .save_manifest(&InstallManifest {
                version: "test".into(),
                game_directory: old.clone(),
                mods_directory: root.join("mods"),
                distribution: Distribution::Standalone,
                installed_files: vec![],
                lovely_owned: true,
                lovely_backup: None,
                steam_app_id_owned: false,
                steam_app_id_backup: None,
                runtime_path: None,
                maintenance_installer: None,
                firewall_installed: false,
                firewall_public: false,
                file_hashes: Default::default(),
                lovely_original_sha256: None,
            })
            .unwrap();
        let inspection = installer.inspect_target(&new);
        assert_eq!(inspection.managed_elsewhere, Some(old));
        assert_eq!(inspection.install_state, "MOVE INSTALLATION");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_guards_restore_prior_files_and_directories() {
        let root = std::env::temp_dir().join(format!("bbt-rollback-{}", rand::random::<u64>()));
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::create_dir_all(root.join("stage")).unwrap();
        std::fs::write(root.join("active/old.txt"), b"old").unwrap();
        std::fs::write(root.join("stage/new.txt"), b"new").unwrap();
        let file = root.join("runtime.exe");
        std::fs::write(&file, b"old runtime").unwrap();
        {
            let _file_guard = FileRollback::replace(&file, b"new runtime").unwrap();
            let _directory_guard = DirectoryRollback::activate(
                &root.join("stage"),
                &root.join("active"),
                &root.join("rollback"),
            )
            .unwrap();
            assert!(root.join("active/new.txt").is_file());
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"old runtime");
        assert!(root.join("active/old.txt").is_file());
        assert!(!root.join("active/new.txt").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_mod_detection_requires_the_installer_marker_and_expected_payload() {
        let root =
            std::env::temp_dir().join(format!("bbt-legacy-detection-{}", rand::random::<u64>()));
        let legacy = root.join("BeatblockTogether");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("README.txt"), b"user-created folder").unwrap();
        assert!(!is_managed_legacy_mod(&legacy));
        fake_managed_legacy_mod(&root);
        assert!(is_managed_legacy_mod(&legacy));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_standalone_install_repair_restore_and_uninstall_round_trip() {
        let root =
            std::env::temp_dir().join(format!("bbt-full-roundtrip-{}", rand::random::<u64>()));
        let game = root.join("Beatblock copy");
        let data = root.join("data");
        let mods = root.join("mods");
        fake_game(&game);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(game.join("version.dll"), b"existing Lovely").unwrap();
        std::fs::write(data.join("runtime.sqlite3"), b"history").unwrap();
        let legacy_runtime = data.join("runtime").join(LEGACY_RUNTIME_FILE_NAME);
        std::fs::create_dir_all(legacy_runtime.parent().unwrap()).unwrap();
        std::fs::write(&legacy_runtime, b"legacy runtime").unwrap();
        let legacy_mod = fake_managed_legacy_mod(&mods);
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());

        let mut progress = Vec::new();
        let manifest = installer
            .install_with_progress_platform(
                Some(game.clone()),
                false,
                Some(Distribution::Standalone),
                false,
                false,
                |event| progress.push(event),
            )
            .unwrap();
        assert_eq!(manifest.distribution, Distribution::Standalone);
        assert!(!manifest.firewall_installed);
        assert!(!legacy_runtime.exists());
        assert!(!legacy_mod.exists());
        assert!(data.join("runtime").join(RUNTIME_FILE_NAME).is_file());
        assert!(mods
            .join("BeatblockOnline/bbt/dashboard_model.lua")
            .is_file());
        let installed_core =
            std::fs::read_to_string(mods.join("BeatblockOnline/bbt/core.lua")).unwrap();
        assert!(installed_core.contains("pendingRequestDeadlineMs"));
        assert!(installed_core.contains("runtime.disconnected"));
        assert!(mods.join("BeatblockOnline/lovely/bootstrap.toml").is_file());
        assert!(file_matches(&game.join("version.dll"), LOVELY_PAYLOAD));
        assert_eq!(
            std::fs::read(game.join("steam_appid.txt")).unwrap(),
            b"3045200\n"
        );
        assert!(manifest.steam_app_id_owned);
        let backup = manifest.lovely_backup.as_ref().unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), b"existing Lovely");
        assert!(progress
            .windows(2)
            .all(|pair| pair[0].percent <= pair[1].percent));
        assert_eq!(progress.iter().filter(|event| event.terminal).count(), 1);

        std::fs::write(
            mods.join("BeatblockOnline/bbt/core.lua"),
            b"stale Online command lifecycle",
        )
        .unwrap();
        std::fs::write(
            data.join("runtime").join(RUNTIME_FILE_NAME),
            b"corrupt runtime",
        )
        .unwrap();
        std::fs::write(game.join("version.dll"), b"corrupt injector").unwrap();
        assert!(installer.inspect_target(&game).repair_required);
        let mut repair_progress = Vec::new();
        installer
            .repair_with_progress_platform(false, |event| repair_progress.push(event))
            .unwrap();
        assert!(repair_progress
            .iter()
            .all(|event| event.operation == OperationKind::Repair));
        assert_eq!(
            repair_progress
                .iter()
                .filter(|event| event.terminal)
                .count(),
            1
        );
        assert!(file_matches(
            &mods.join("BeatblockOnline/bbt/core.lua"),
            include_bytes!("../../mod/shared/bbt/core.lua")
        ));
        assert!(file_matches(
            &data.join("runtime").join(RUNTIME_FILE_NAME),
            RUNTIME_PAYLOAD
        ));
        assert!(file_matches(&game.join("version.dll"), LOVELY_PAYLOAD));
        assert_eq!(std::fs::read(backup).unwrap(), b"existing Lovely");

        let fake_obs = root.join("obs-studio");
        std::fs::create_dir_all(fake_obs.join("bin/64bit")).unwrap();
        let installed_obs = installer
            .install_obs_plugin_into(fake_obs, &root.join("ProgramData"))
            .unwrap();
        assert!(file_matches(&installed_obs, OBS_PLUGIN_PAYLOAD));
        assert!(installer.obs_plugin_ready());

        installer.restore_with_progress(|_| {}).unwrap();
        assert!(!mods.join("BeatblockOnline").exists());
        assert!(!game.join("steam_appid.txt").exists());
        assert_eq!(
            std::fs::read(game.join("version.dll")).unwrap(),
            b"existing Lovely"
        );

        install_isolated(
            &installer,
            &game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        )
        .unwrap();
        installer
            .uninstall_with_progress_platform(false, false, |_| {})
            .unwrap();
        assert!(!mods.join("BeatblockOnline").exists());
        assert!(!game.join("steam_appid.txt").exists());
        assert_eq!(
            std::fs::read(game.join("version.dll")).unwrap(),
            b"existing Lovely"
        );
        assert!(!data.join("install-manifest.json").exists());
        assert!(!data.join("runtime").join(RUNTIME_FILE_NAME).exists());
        assert!(!legacy_runtime.exists());
        // The test-only custom ProgramData root is deliberately outside the
        // production cleanup allowlist, so an elevated uninstall leaves it.
        assert!(installed_obs.exists());
        assert!(data.join("runtime.sqlite3").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_transaction_rolls_back_after_lovely_replacement_failure() {
        let root =
            std::env::temp_dir().join(format!("bbt-full-rollback-{}", rand::random::<u64>()));
        let game = root.join("game");
        let data = root.join("data");
        let mods = root.join("mods");
        fake_game(&game);
        std::fs::create_dir_all(game.join("version.dll")).unwrap();
        std::fs::create_dir_all(mods.join("BeatblockOnline")).unwrap();
        std::fs::write(mods.join("BeatblockOnline/previous.txt"), b"previous mod").unwrap();
        let legacy_mod = fake_managed_legacy_mod(&mods);
        let runtime = data.join("runtime").join(LEGACY_RUNTIME_FILE_NAME);
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&runtime, b"previous runtime").unwrap();
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());

        let result = install_isolated(
            &installer,
            &game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&runtime).unwrap(), b"previous runtime");
        assert!(legacy_mod.is_dir());
        assert_eq!(
            std::fs::read(mods.join("BeatblockOnline/previous.txt")).unwrap(),
            b"previous mod"
        );
        assert!(!mods
            .join("BeatblockOnline/bbt/dashboard_model.lua")
            .exists());
        assert!(!data.join("install-manifest.json").exists());
        assert!(!std::fs::read_dir(&game)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("version.dll.")
            }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn move_installation_and_adapter_detection_are_exclusive() {
        let root = std::env::temp_dir().join(format!("bbt-full-move-{}", rand::random::<u64>()));
        let old_game = root.join("old game");
        let new_game = root.join("new game");
        let data = root.join("data");
        let mods = root.join("mods");
        fake_game(&old_game);
        fake_game(&new_game);
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());

        install_isolated(
            &installer,
            &old_game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(old_game.join("version.dll").is_file());
        assert!(old_game.join("steam_appid.txt").is_file());
        install_isolated(
            &installer,
            &new_game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!old_game.join("version.dll").exists());
        assert!(!old_game.join("steam_appid.txt").exists());
        assert!(new_game.join("version.dll").is_file());
        assert!(new_game.join("steam_appid.txt").is_file());
        assert_eq!(
            installer.load_manifest().unwrap().unwrap().game_directory,
            new_game
        );

        std::fs::create_dir_all(mods.join("BeatblockPlus")).unwrap();
        std::fs::write(
            mods.join("BeatblockPlus/mod.json"),
            r#"{"id":"beatblock-plus","version":"2.1.0"}"#,
        )
        .unwrap();
        let wrong_adapter = install_isolated(
            &installer,
            &new_game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        );
        assert!(wrong_adapter
            .unwrap_err()
            .to_string()
            .contains("avoid loading both BBT adapters"));
        let plus = install_isolated(&installer, &new_game, None, &mut Vec::new()).unwrap();
        assert_eq!(plus.distribution, Distribution::BeatblockPlus);
        assert!(mods.join("BeatblockOnline/mod.json").is_file());
        assert!(!mods.join("BeatblockOnline/lovely/bootstrap.toml").exists());
        installer
            .uninstall_with_progress_platform(true, false, |_| {})
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compatible_existing_lovely_is_preserved_and_direct_launch_is_managed() {
        let root =
            std::env::temp_dir().join(format!("bbt-compatible-lovely-{}", rand::random::<u64>()));
        let game = root.join("game");
        let data = root.join("data");
        let mods = root.join("mods");
        fake_game(&game);
        let mut compatible = b"MZ".to_vec();
        compatible.extend_from_slice(b"third-party signed lovely-injector fixture");
        std::fs::write(game.join("version.dll"), &compatible).unwrap();
        std::fs::write(game.join("steam_appid.txt"), b"999\n").unwrap();
        let installer = Installer::with_mods_directory(data, mods);

        let manifest = install_isolated(
            &installer,
            &game,
            Some(Distribution::Standalone),
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(std::fs::read(game.join("version.dll")).unwrap(), compatible);
        assert!(!manifest.lovely_owned);
        assert!(manifest
            .lovely_backup
            .as_ref()
            .is_some_and(|path| path.is_file()));
        assert!(manifest.steam_app_id_owned);
        assert!(manifest
            .steam_app_id_backup
            .as_ref()
            .is_some_and(|path| path.is_file()));
        assert_eq!(
            std::fs::read(game.join("steam_appid.txt")).unwrap(),
            b"3045200\n"
        );
        installer
            .uninstall_with_progress_platform(false, false, |_| {})
            .unwrap();
        assert_eq!(std::fs::read(game.join("version.dll")).unwrap(), compatible);
        assert_eq!(
            std::fs::read(game.join("steam_appid.txt")).unwrap(),
            b"999\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_preserves_data_unless_explicitly_requested() {
        let root = std::env::temp_dir().join(format!("bbt-uninstall-{}", rand::random::<u64>()));
        let data = root.join("data");
        let mods = root.join("mods");
        let game = root.join("game");
        std::fs::create_dir_all(mods.join("BeatblockOnline")).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("runtime.sqlite3"), b"history").unwrap();
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());
        let manifest = InstallManifest {
            version: "test".into(),
            game_directory: game.clone(),
            mods_directory: mods.clone(),
            distribution: Distribution::Standalone,
            installed_files: Vec::new(),
            lovely_owned: false,
            lovely_backup: None,
            steam_app_id_owned: false,
            steam_app_id_backup: None,
            runtime_path: None,
            maintenance_installer: None,
            firewall_installed: false,
            firewall_public: false,
            file_hashes: Default::default(),
            lovely_original_sha256: None,
        };
        installer.save_manifest(&manifest).unwrap();
        installer.uninstall_with_data(false).unwrap();
        assert!(data.join("runtime.sqlite3").is_file());
        installer.save_manifest(&manifest).unwrap();
        installer.uninstall_with_data(true).unwrap();
        assert!(!data.join("runtime.sqlite3").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_utf8_bom_manifest_is_migrated() {
        let root = std::env::temp_dir().join(format!("bbt-bom-manifest-{}", rand::random::<u64>()));
        let installer = Installer::with_mods_directory(root.clone(), root.join("mods"));
        let manifest = InstallManifest {
            version: "legacy".into(),
            game_directory: root.join("game"),
            mods_directory: root.join("mods"),
            distribution: Distribution::Standalone,
            installed_files: vec![],
            lovely_owned: false,
            lovely_backup: None,
            steam_app_id_owned: false,
            steam_app_id_backup: None,
            runtime_path: None,
            maintenance_installer: None,
            firewall_installed: false,
            firewall_public: false,
            file_hashes: Default::default(),
            lovely_original_sha256: None,
        };
        std::fs::create_dir_all(&root).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(serde_json::to_vec(&manifest).unwrap());
        std::fs::write(installer.manifest_path(), bytes).unwrap();
        assert_eq!(
            installer.load_manifest().unwrap().unwrap().version,
            "legacy"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn forged_manifest_cannot_escape_installer_owned_paths() {
        let root =
            std::env::temp_dir().join(format!("bbt-forged-manifest-{}", rand::random::<u64>()));
        let data = root.join("data");
        let mods = root.join("mods");
        let game = root.join("game");
        fake_game(&game);
        std::fs::create_dir_all(&data).unwrap();
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());
        let mut manifest = InstallManifest {
            version: "test".into(),
            game_directory: game.clone(),
            mods_directory: mods,
            distribution: Distribution::Standalone,
            installed_files: Vec::new(),
            lovely_owned: false,
            lovely_backup: None,
            steam_app_id_owned: false,
            steam_app_id_backup: None,
            runtime_path: Some(data.join("runtime").join(RUNTIME_FILE_NAME)),
            maintenance_installer: Some(
                data.join("installer").join("BeatblockOnlineInstaller.exe"),
            ),
            firewall_installed: false,
            firewall_public: false,
            file_hashes: Default::default(),
            lovely_original_sha256: None,
        };

        manifest.lovely_backup = Some(root.join("victim.dll"));
        std::fs::write(
            installer.manifest_path(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(installer.load_manifest().is_err());

        manifest.lovely_backup = None;
        manifest.runtime_path = Some(root.join("victim.exe"));
        std::fs::write(
            installer.manifest_path(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(installer.load_manifest().is_err());

        manifest.runtime_path = None;
        manifest
            .file_hashes
            .insert(PathBuf::from("../victim"), "00".repeat(32));
        std::fs::write(
            installer.manifest_path(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(installer.load_manifest().is_err());

        std::fs::write(
            installer.manifest_path(),
            vec![b' '; MAX_INSTALL_MANIFEST_BYTES as usize + 1],
        )
        .unwrap();
        assert!(installer.load_manifest().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn forged_obs_marker_cannot_delete_arbitrary_files() {
        let root = std::env::temp_dir().join(format!("bbt-forged-obs-{}", rand::random::<u64>()));
        let data = root.join("data");
        let mods = root.join("mods");
        let game = root.join("game");
        fake_game(&game);
        let victim_plugin = root.join("victim.dll");
        let victim_locale = root.join("victim.ini");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(&victim_plugin, b"keep").unwrap();
        std::fs::write(&victim_locale, b"keep").unwrap();
        let installer = Installer::with_mods_directory(data.clone(), mods.clone());
        installer
            .save_manifest(&InstallManifest {
                version: "test".into(),
                game_directory: game,
                mods_directory: mods,
                distribution: Distribution::Standalone,
                installed_files: Vec::new(),
                lovely_owned: false,
                lovely_backup: None,
                steam_app_id_owned: false,
                steam_app_id_backup: None,
                runtime_path: None,
                maintenance_installer: None,
                firewall_installed: false,
                firewall_public: false,
                file_hashes: Default::default(),
                lovely_original_sha256: None,
            })
            .unwrap();
        let forged = ObsInstallManifest {
            version: "test".into(),
            obs_directory: root.clone(),
            plugin: victim_plugin.clone(),
            locale: victim_locale.clone(),
            plugin_sha256: "00".repeat(32),
        };
        std::fs::write(
            data.join("obs-install.json"),
            serde_json::to_vec(&forged).unwrap(),
        )
        .unwrap();

        installer
            .uninstall_with_progress_platform(false, false, |_| {})
            .unwrap();
        assert!(victim_plugin.is_file());
        assert!(victim_locale.is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn every_lovely_module_is_embedded_in_both_payloads() {
        let hooks =
            std::str::from_utf8(include_bytes!("../../mod/shared/lovely/hooks.toml")).unwrap();
        let declared = hooks
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("source = \"")
                    .and_then(|v| v.strip_suffix('"'))
            })
            .collect::<Vec<_>>();
        let embedded = SHARED_MOD_PAYLOAD
            .iter()
            .map(|(path, _)| *path)
            .collect::<Vec<_>>();
        assert!(!declared.is_empty());
        for source in declared {
            assert!(
                embedded.contains(&source),
                "Lovely module {source} is not in the installer payload"
            );
        }
        assert!(embedded.contains(&"bbt/dashboard_model.lua"));
    }

    #[test]
    fn embedded_mod_payload_contains_online_recovery_contracts() {
        let root =
            std::env::temp_dir().join(format!("bbt-recovery-contract-{}", rand::random::<u64>()));
        for (relative, bytes) in SHARED_MOD_PAYLOAD {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        validate_online_recovery_contract(&root).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn progress_is_monotonic_and_has_one_terminal_result() {
        let mut events = vec![
            OperationProgress::step(OperationKind::Install, "validation", 3, "validate"),
            OperationProgress::step(OperationKind::Install, "staging", 30, "stage"),
            OperationProgress::complete(OperationKind::Install, "done"),
        ];
        assert!(events
            .windows(2)
            .all(|pair| pair[0].percent <= pair[1].percent));
        assert_eq!(events.drain(..).filter(|event| event.terminal).count(), 1);
    }

    #[test]
    fn elevated_status_reader_observes_a_terminal_error_written_at_process_exit() {
        let root =
            std::env::temp_dir().join(format!("bbt-elevated-status-{}", rand::random::<u64>()));
        let status = root.join("operation.json");
        let event = OperationProgress {
            operation: OperationKind::Install,
            phase: "failed".into(),
            percent: 100,
            message: "specific elevated helper failure".into(),
            severity: Severity::Error,
            terminal: true,
        };
        write_operation_status(&status, &event).unwrap();
        let observed = read_operation_progress(&status).unwrap();
        assert!(observed.terminal);
        assert_eq!(observed.severity, Severity::Error);
        assert_eq!(observed.message, event.message);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn running_process_detection_finds_the_current_test_executable() {
        let executable = std::env::current_exe().unwrap();
        let name = executable.file_name().unwrap().to_string_lossy();
        assert!(process_is_running(&name).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn firewall_command_targets_runtime_and_selected_profiles() {
        let runtime = Path::new(r"C:\Program Files\Beatblock Online/Runtime.exe");
        let private = firewall_command(runtime, false, true)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(private.contains(&r"program=C:\Program Files\Beatblock Online\Runtime.exe".into()));
        assert!(private
            .iter()
            .find(|arg| arg.starts_with("program="))
            .is_some_and(|arg| !arg.contains('/')));
        assert!(private.contains(&"profile=private,domain".into()));

        let public = firewall_command(runtime, true, true)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(public.contains(&"profile=private,public".into()));

        let remove = firewall_command(runtime, false, false)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(remove.contains(&"delete".into()));
        assert!(!remove.iter().any(|arg| arg.starts_with("program=")));
    }

    #[test]
    fn legacy_manifest_defaults_to_private_firewall_profile() {
        let value = serde_json::json!({
            "version": "legacy",
            "gameDirectory": "C:\\Beatblock",
            "modsDirectory": "C:\\Mods",
            "distribution": "standalone",
            "installedFiles": [],
            "lovelyOwned": true,
            "lovelyBackup": null,
            "firewallInstalled": true
        });
        let manifest: InstallManifest = serde_json::from_value(value).unwrap();
        assert!(!manifest.firewall_public);
    }

    #[test]
    fn embedded_obs_source_is_a_real_module_with_required_exports() {
        validate_obs_payload(OBS_PLUGIN_PAYLOAD).unwrap();
        assert!(OBS_PLUGIN_PAYLOAD.len() > 8_000);
        let source = include_str!("../../obs-plugin/src/plugin.c");
        assert!(source.contains(r"BeatblockOnline\\BeatblockOnline\\data\\render-streams"));
        assert!(source.contains("beatblock_online_player_stream"));
        let stale_cleanup = source
            .split("static void clear_stale_resources")
            .nth(1)
            .and_then(|source| source.split("static const char").next())
            .expect("OBS source contains stale-texture cleanup");
        assert!(
            !stale_cleanup.contains("ctx->sequence = 0"),
            "stale cleanup must not republish and recreate the abandoned texture"
        );
        assert!(stale_cleanup.contains("retry_frame_mapping_later(ctx)"));
        assert!(stale_cleanup.contains("clear_video_frame(ctx)"));
        let frame_cleanup = source
            .split("static void clear_video_frame")
            .nth(1)
            .and_then(|source| source.split("static void clear_stale_resources").next())
            .expect("OBS source contains frame-buffer cleanup");
        assert!(
            frame_cleanup.contains("ctx->pixel_capacity = 0"),
            "stale cleanup must release its retained CPU frame buffer"
        );
        let video_render = source
            .split("static void video_render")
            .nth(1)
            .and_then(|source| source.split("static struct obs_source_info").next())
            .expect("OBS source contains its render callback");
        assert!(
            video_render
                .contains("obs_source_draw(ctx->texture, 0, 0, ctx->width, ctx->height, false)"),
            "single-texture sources must use the supported OBS draw path"
        );
        assert!(source.contains("beatblock_online_audio"));
        assert!(source.contains("OBS_SOURCE_VIDEO | OBS_SOURCE_SRGB"));
        assert!(!source.contains("OBS_SOURCE_CUSTOM_DRAW"));
    }

    #[test]
    fn invalid_obs_payloads_are_rejected_before_installation() {
        assert!(validate_obs_payload(&[]).is_err());
        assert!(validate_obs_payload(&vec![b'X'; 8192]).is_err());
    }

    #[test]
    fn obs_uses_recommended_program_data_layout() {
        let (plugin, locale) = obs_program_data_paths(Path::new(r"C:\ProgramData"));
        assert_eq!(
            plugin,
            PathBuf::from(
                r"C:\ProgramData\obs-studio\plugins\beatblock-online-obs\bin\64bit\beatblock-online-obs.dll"
            )
        );
        assert_eq!(
            locale,
            PathBuf::from(
                r"C:\ProgramData\obs-studio\plugins\beatblock-online-obs\data\locale\en-US.ini"
            )
        );
    }

    #[test]
    fn portable_obs_uses_selected_local_plugin_layout() {
        let root =
            std::env::temp_dir().join(format!("bbt-portable-plugin-{}", rand::random::<u64>()));
        let data = root.join("data");
        let obs = root.join("Portable OBS");
        let executable = obs.join("bin/64bit/obs64.exe");
        let program_data = root.join("ProgramData");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(executable, b"portable OBS fixture").unwrap();
        std::fs::write(obs.join("portable_mode.txt"), b"").unwrap();
        let installer = Installer::with_mods_directory(data.clone(), root.join("mods"));

        let installed = installer
            .install_obs_plugin_into(obs.clone(), &program_data)
            .unwrap();
        let (plugin, locale) = obs_portable_paths(&obs);
        assert_eq!(installed, plugin);
        assert!(file_matches(&plugin, OBS_PLUGIN_PAYLOAD));
        assert!(file_matches(
            &locale,
            include_bytes!("../../obs-plugin/data/locale/en-US.ini")
        ));
        assert!(!obs_program_data_paths(&program_data).0.exists());
        let record: ObsInstallManifest =
            serde_json::from_slice(&std::fs::read(data.join("obs-install.json")).unwrap()).unwrap();
        assert!(obs_record_paths_are_managed(&record));
        assert!(installer.obs_plugin_ready());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manual_obs_location_accepts_portable_root_and_nearby_paths() {
        let root = std::env::temp_dir().join(format!("bbt-portable-obs-{}", rand::random::<u64>()));
        let executable = root.join("bin/64bit/obs64.exe");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"portable OBS fixture").unwrap();
        let installer = Installer::with_mods_directory(root.join("data"), root.join("mods"));

        for selected in [
            root.clone(),
            root.join("bin"),
            root.join("bin/64bit"),
            executable,
        ] {
            installer.set_obs_directory(Some(selected)).unwrap();
            assert_eq!(installer.obs_directory(), Some(root.clone()));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_manual_obs_location_does_not_fall_back_to_another_installation() {
        let root = std::env::temp_dir().join(format!("bbt-invalid-obs-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let installer = Installer::with_mods_directory(root.join("data"), root.join("mods"));
        installer.set_obs_directory(Some(root.clone())).unwrap();
        assert_eq!(installer.obs_directory(), None);
        assert!(!installer.obs_plugin_available());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_custom_obs_installation_is_rediscovered_from_its_marker() {
        let root = std::env::temp_dir().join(format!("bbt-recorded-obs-{}", rand::random::<u64>()));
        let data = root.join("data");
        let obs = root.join("Portable OBS");
        std::fs::create_dir_all(obs.join("bin/64bit")).unwrap();
        std::fs::write(obs.join("bin/64bit/obs64.exe"), b"portable OBS fixture").unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let record = ObsInstallManifest {
            version: "test".into(),
            obs_directory: obs.clone(),
            plugin: root.join("plugin.dll"),
            locale: root.join("en-US.ini"),
            plugin_sha256: "00".repeat(32),
        };
        std::fs::write(
            data.join("obs-install.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();

        let installer = Installer::with_mods_directory(data, root.join("mods"));
        assert_eq!(installer.obs_directory(), Some(obs));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn staged_obs_install_rolls_back_until_core_transaction_commits() {
        let root = std::env::temp_dir().join(format!("bbt-obs-atomic-{}", rand::random::<u64>()));
        let data = root.join("data");
        let obs = root.join("obs-studio");
        let program_data = root.join("ProgramData");
        std::fs::create_dir_all(obs.join("bin/64bit")).unwrap();
        let installer = Installer::with_mods_directory(data.clone(), root.join("mods"));
        let (plugin, _) = obs_program_data_paths(&program_data);
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(&plugin, b"previous plugin").unwrap();
        let legacy = program_data
            .join("obs-studio/plugins/beatblock-together-obs/bin/64bit/beatblock-together-obs.dll");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"previous legacy plugin").unwrap();

        let transaction = installer.stage_obs_plugin_into(obs, &program_data).unwrap();
        assert!(file_matches(&plugin, OBS_PLUGIN_PAYLOAD));
        assert!(!legacy.exists());
        drop(transaction);
        assert_eq!(std::fs::read(&plugin).unwrap(), b"previous plugin");
        assert_eq!(std::fs::read(&legacy).unwrap(), b"previous legacy plugin");
        assert!(!data.join("obs-install.json").exists());

        installer
            .install_obs_plugin_into(root.join("obs-studio"), &program_data)
            .unwrap();
        assert!(file_matches(&plugin, OBS_PLUGIN_PAYLOAD));
        assert!(!legacy.exists());
        assert!(data.join("obs-install.json").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn verified_obs_component_does_not_report_a_stale_failure() {
        let root = std::env::temp_dir().join(format!("bbt-obs-state-{}", rand::random::<u64>()));
        let data = root.join("data");
        let plugin = root.join("beatblock-online-obs.dll");
        let locale = root.join("en-US.ini");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(&plugin, OBS_PLUGIN_PAYLOAD).unwrap();
        std::fs::write(
            &locale,
            include_bytes!("../../obs-plugin/data/locale/en-US.ini"),
        )
        .unwrap();
        let marker = ObsInstallManifest {
            version: env!("CARGO_PKG_VERSION").into(),
            obs_directory: root.clone(),
            plugin,
            locale,
            plugin_sha256: hex::encode(Sha256::digest(OBS_PLUGIN_PAYLOAD)),
        };
        std::fs::write(
            data.join("obs-install.json"),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();

        let installer = Installer::with_mods_directory(data, root.join("mods"));
        let inspection = installer.inspect_target(&root.join("game"));
        let obs = inspection
            .components
            .iter()
            .find(|component| component.name == "OBS video/audio plugin")
            .unwrap();
        assert_eq!(obs.state, ComponentState::Ready);
        assert_eq!(obs.label, "Installed");
        assert_eq!(obs.details, "Installed and hash verified");
        let _ = std::fs::remove_dir_all(root);
    }
}
