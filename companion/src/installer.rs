use crate::mod_payload::SHARED_MOD_PAYLOAD;
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

static LOVELY_PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lovely-version.dll"));
static OBS_PLUGIN_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/beatblock-together-obs.dll"));
static RUNTIME_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/BeatblockTogetherRuntime.exe"));

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
    pub supported_build: bool,
    pub fingerprint: Option<String>,
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
    pub supported_build: bool,
    pub runtime_bundled: bool,
    pub lovely_bundled: bool,
    pub obs_plugin_bundled: bool,
    pub firewall_installed: bool,
    pub message: String,
}

pub struct Installer {
    data_dir: PathBuf,
    mods_directory_override: Option<PathBuf>,
}

impl Installer {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            mods_directory_override: None,
        }
    }

    #[cfg(test)]
    fn with_mods_directory(data_dir: PathBuf, mods_directory: PathBuf) -> Self {
        Self {
            data_dir,
            mods_directory_override: Some(mods_directory),
        }
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
        let fingerprint = valid
            .then(|| sha256_file(&selected.join("Beatblock.exe")).ok())
            .flatten();
        let supported_build = fingerprint.as_deref() == Some(supported_game_hash());
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
        let mod_dir = mods_directory.join("BeatblockTogether");
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
        let runtime_present = manifest
            .as_ref()
            .and_then(|m| m.runtime_path.as_ref())
            .is_some_and(|p| p.is_file() && file_matches(p, RUNTIME_PAYLOAD));
        let renderer_ready = self
            .data_dir
            .join(
                "renderer-profile/Beatblock/Mods/BeatblockTogetherRenderer/bbt/dashboard_model.lua",
            )
            .is_file();
        let backup_warning = manifest
            .as_ref()
            .and_then(|m| m.lovely_backup.as_ref())
            .is_some_and(|p| {
                p.is_file()
                    && lovely_path.is_file()
                    && sha256_file(p).ok() == sha256_file(&lovely_path).ok()
            });
        let required_ready =
            valid && managed_here && adapter_ok && shared_ok && lovely_present && runtime_present;
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
            } else if supported_build {
                ComponentState::Ready
            } else {
                ComponentState::Attention
            },
            if !valid {
                "Invalid"
            } else if supported_build {
                "Supported"
            } else {
                "Uncertified"
            },
            "—",
            fingerprint
                .as_deref()
                .map(short_hash)
                .unwrap_or_else(|| validation_message.clone()),
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
        } else if lovely_matches {
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
            } else {
                "Existing"
            },
            "Yes",
            if backup_warning {
                "Legacy backup matches injector; backup preserved"
            } else if lovely_matches {
                "Bundled no-console build"
            } else {
                "Existing Lovely build will be preserved"
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
        let obs = self.data_dir.join("obs-install.json").is_file();
        components.push(component(
            "OBS plugin",
            if obs {
                ComponentState::Ready
            } else {
                ComponentState::Optional
            },
            if obs { "Installed" } else { "Optional" },
            "Conditional",
            if OBS_PLUGIN_PAYLOAD.is_empty() {
                "Not included in this build"
            } else {
                "Install when OBS is detected"
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
            supported_build,
            fingerprint,
            distribution,
            install_state: state.into(),
            managed_elsewhere,
            repair_required,
            components,
            message: if valid {
                "This selected folder will be modified. Online competition requires a certified build.".into()
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
        let supported_build = game_directory
            .as_ref()
            .and_then(|path| sha256_file(&path.join("Beatblock.exe")).ok())
            .is_some_and(|hash| hash == supported_game_hash());
        let runtime_present = manifest
            .as_ref()
            .and_then(|value| value.runtime_path.as_ref())
            .is_some_and(|path| path.is_file());
        let obs_plugin_present = self.data_dir.join("obs-install.json").is_file();
        InstallStatus {
            game_directory: game_directory.clone(),
            installed: manifest.is_some(),
            distribution: manifest.as_ref().map(|manifest| manifest.distribution),
            lovely_present,
            beatblock_plus_present,
            runtime_present,
            obs_plugin_present,
            supported_build,
            runtime_bundled: !RUNTIME_PAYLOAD.is_empty(),
            lovely_bundled: !LOVELY_PAYLOAD.is_empty(),
            obs_plugin_bundled: !OBS_PLUGIN_PAYLOAD.is_empty(),
            firewall_installed: manifest
                .as_ref()
                .is_some_and(|value| value.firewall_installed),
            message: if supported_build {
                "Supported Beatblock build detected".into()
            } else {
                "Beatblock was not found or its build is not certified for competition".into()
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
        mut progress: F,
    ) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        progress(OperationProgress::step(
            OperationKind::Install,
            "validation",
            3,
            "Validating the selected Beatblock folder",
        ));
        if RUNTIME_PAYLOAD.is_empty() {
            bail!("this installer build does not contain BeatblockTogetherRuntime.exe");
        }
        let game_directory = explicit_game_directory
            .or_else(|| self.initial_game_directory())
            .context("Beatblock was not found; choose the folder containing Beatblock.exe")?;
        validate_game_directory(&game_directory)?;
        let managed_manifest = self.load_manifest()?;
        let hash = sha256_file(&game_directory.join("Beatblock.exe"))?;
        if hash != supported_game_hash() && !allow_unknown_build {
            bail!("this Beatblock build is not certified; repair or update Beatblock first");
        }
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
        let mod_directory = mods_directory.join("BeatblockTogether");
        let stage_directory =
            mods_directory.join(format!(".BeatblockTogether.stage-{}", uuid::Uuid::new_v4()));
        let rollback_directory = mods_directory.join(format!(
            ".BeatblockTogether.rollback-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(stage_directory.join("bbt"))?;
        std::fs::create_dir_all(stage_directory.join("lovely"))?;
        let mut installed_files = Vec::new();
        let runtime_path = self.data_dir.join("runtime/BeatblockTogetherRuntime.exe");
        progress(OperationProgress::step(
            OperationKind::Install,
            "runtime",
            20,
            "Staging the hidden online runtime",
        ));
        let mut runtime_rollback = FileRollback::replace(&runtime_path, RUNTIME_PAYLOAD)
            .context("install hidden runtime; exit Online before updating")?;
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
                    b"Beatblock Together standalone Lovely package. Installed by BeatblockTogetherInstaller.exe.\n",
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
        if !LOVELY_PAYLOAD.is_empty() {
            if lovely_target.is_file()
                && lovely_backup.as_ref().is_none_or(|p| !p.is_file())
                && !file_matches(&lovely_target, LOVELY_PAYLOAD)
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
            lovely_rollback = Some(FileRollback::replace(&lovely_target, LOVELY_PAYLOAD)?);
            installed_files.push(lovely_target.clone());
        } else if !lovely_target.is_file() {
            bail!("the release is missing its bundled no-console Lovely payload");
        }

        let maintenance_installer = self
            .data_dir
            .join("installer/BeatblockTogetherInstaller.exe");
        if let Some(parent) = maintenance_installer.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let current_exe = std::env::current_exe()?;
        if current_exe != maintenance_installer {
            std::fs::copy(&current_exe, &maintenance_installer)?;
        }
        write(
            &mod_directory.join("installer-path.txt"),
            maintenance_installer.to_string_lossy().as_bytes(),
            &mut installed_files,
        )?;
        progress(OperationProgress::step(
            OperationKind::Install,
            "system_changes",
            76,
            "Applying the program-scoped firewall rule",
        ));
        Self::configure_firewall(&runtime_path, firewall_public, true)?;
        let firewall_installed = true;
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
            runtime_path: Some(runtime_path.clone()),
            maintenance_installer: Some(maintenance_installer),
            firewall_installed,
            firewall_public,
            file_hashes,
            lovely_original_sha256,
        };
        self.register_uninstall(&manifest)?;
        validate_staged_payload(&mod_directory, distribution)?;
        if !file_matches(&runtime_path, RUNTIME_PAYLOAD)
            || !file_matches(&lovely_target, LOVELY_PAYLOAD)
        {
            bail!("post-install verification failed: runtime or Lovely hash differs from the bundled payload");
        }
        // A move restores the former target only after the new target passes
        // every check. Keep a rollback guard until the new manifest is durable.
        let mut previous_target_rollback = None;
        if let Some(old) = managed_manifest
            .as_ref()
            .filter(|old| old.game_directory != game_directory)
        {
            let old_lovely = old.game_directory.join("version.dll");
            if let Some(backup) = old.lovely_backup.as_ref().filter(|path| path.is_file()) {
                previous_target_rollback =
                    Some(FileRollback::replace(&old_lovely, &std::fs::read(backup)?)?);
            } else if old.lovely_owned
                && old_lovely.is_file()
                && !other_lovely_mods(&old.mods_directory)
            {
                previous_target_rollback = Some(FileRollback::remove(&old_lovely)?);
            }
        }
        self.save_manifest(&manifest)?;
        mod_rollback.commit()?;
        runtime_rollback.commit();
        if let Some(rollback) = lovely_rollback.as_mut() {
            rollback.commit();
        }
        if let Some(rollback) = previous_target_rollback.as_mut() {
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

    pub fn repair_with_progress<F>(&self, mut progress: F) -> Result<InstallManifest>
    where
        F: FnMut(OperationProgress),
    {
        let manifest = self
            .load_manifest()?
            .context("Beatblock Together is not installed")?;
        progress(OperationProgress::step(
            OperationKind::Repair,
            "validation",
            2,
            "Inspecting managed components",
        ));
        let allow_unknown = sha256_file(&manifest.game_directory.join("Beatblock.exe"))
            .ok()
            .as_deref()
            != Some(supported_game_hash());
        let result = self.install_with_progress_options(
            Some(manifest.game_directory),
            allow_unknown,
            Some(manifest.distribution),
            manifest.firewall_public,
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
        progress(OperationProgress::step(
            OperationKind::Restore,
            "validation",
            5,
            "Reading the installation manifest",
        ));
        let manifest = self
            .load_manifest()?
            .context("Beatblock Together is not installed")?;
        let mod_directory = manifest.mods_directory.join("BeatblockTogether");
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
        progress(OperationProgress::complete(
            OperationKind::Restore,
            "Game files were restored",
        ));
        Ok(())
    }

    pub fn set_firewall_profile(&self, public: bool) -> Result<()> {
        let mut manifest = self
            .load_manifest()?
            .context("Beatblock Together is not installed")?;
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
        let directory = profile.join("Beatblock/Mods/BeatblockTogetherRenderer");
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
        if OBS_PLUGIN_PAYLOAD.is_empty() {
            bail!("this installer build does not contain the OBS plugin payload");
        }
        let program_files =
            std::env::var_os("ProgramFiles").context("Program Files is unavailable")?;
        let obs = PathBuf::from(program_files).join("obs-studio");
        if !obs.is_dir() {
            bail!("OBS Studio was not detected");
        }
        let plugin = obs.join("obs-plugins/64bit/beatblock-together-obs.dll");
        let locale = obs.join("data/obs-plugins/beatblock-together-obs/locale/en-US.ini");
        if let Some(parent) = plugin.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = locale.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&plugin, OBS_PLUGIN_PAYLOAD)?;
        std::fs::write(
            &locale,
            include_bytes!("../../obs-plugin/data/locale/en-US.ini"),
        )?;
        std::fs::write(
            self.data_dir.join("obs-install.json"),
            serde_json::to_vec_pretty(&json_paths(&plugin, &locale))?,
        )?;
        Ok(plugin)
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
        let app_id_path = selected.join("steam_appid.txt");
        let prior_app_id = std::fs::read(&app_id_path).ok();
        if prior_app_id.is_none() {
            std::fs::write(&app_id_path, b"3045200\n")?;
        }
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
        let log_directory = self
            .mods_directory()
            .context("Windows APPDATA is unavailable")?
            .join("lovely/log");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut latest_log = None;
        let mut last_text = String::new();
        let result = loop {
            if let Some(status) = child.try_wait()? {
                break Err(anyhow::anyhow!(
                    "Beatblock exited during startup ({status}). {}",
                    lovely_error_excerpt(&last_text)
                ));
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
        if let Some(bytes) = prior_app_id {
            std::fs::write(&app_id_path, bytes)?;
        } else {
            let _ = std::fs::remove_file(&app_id_path);
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

    pub fn uninstall_with_progress<F>(&self, remove_user_data: bool, mut progress: F) -> Result<()>
    where
        F: FnMut(OperationProgress),
    {
        progress(OperationProgress::step(
            OperationKind::Uninstall,
            "validation",
            5,
            "Reading the managed installation",
        ));
        let Some(manifest) = self.load_manifest()? else {
            progress(OperationProgress::complete(
                OperationKind::Uninstall,
                "Beatblock Together is already uninstalled",
            ));
            return Ok(());
        };
        let mod_directory = manifest.mods_directory.join("BeatblockTogether");
        if mod_directory.exists() {
            progress(OperationProgress::step(
                OperationKind::Uninstall,
                "mod_payload",
                25,
                "Removing the in-game adapter and shared payload",
            ));
            std::fs::remove_dir_all(mod_directory)?;
        }
        let lovely = manifest.game_directory.join("version.dll");
        if let Some(backup) = manifest.lovely_backup {
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
            let _ = Self::configure_firewall(runtime, false, false);
            let _ = std::fs::remove_file(runtime);
        }
        let _ = self.unregister_uninstall();
        if let Ok(bytes) = std::fs::read(self.data_dir.join("obs-install.json")) {
            if let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(&bytes) {
                for path in paths {
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
            Foundation::{CloseHandle, WAIT_OBJECT_0},
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
            bail!("administrator approval was cancelled or Windows could not start the helper: {error}");
        }
        let mut last = None;
        let mut last_event = None;
        loop {
            if let Ok(bytes) = std::fs::read(status_path) {
                if let Ok(event) = serde_json::from_slice::<OperationProgress>(&bytes) {
                    if last != Some((event.percent, event.phase.clone())) {
                        last = Some((event.percent, event.phase.clone()));
                        last_event = Some(event.clone());
                        progress(event);
                    }
                }
            }
            let wait = unsafe { WaitForSingleObject(info.hProcess, 200) };
            if wait == WAIT_OBJECT_0 {
                break;
            }
        }
        let mut code = 1u32;
        unsafe {
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
        }
        if code != 0 {
            let detail = last_event
                .as_ref()
                .filter(|event| event.terminal && event.severity == Severity::Error)
                .map(|event| event.message.as_str());
            let _ = std::fs::remove_file(status_path);
            if let Some(detail) = detail {
                bail!("{detail}");
            }
            bail!("administrator helper failed with exit code {code}");
        }
        let _ = std::fs::remove_file(status_path);
        Ok(())
    }

    fn register_uninstall(&self, manifest: &InstallManifest) -> Result<()> {
        #[cfg(windows)]
        {
            let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\BeatblockTogether";
            let current = std::env::current_exe()?;
            let exe = manifest
                .maintenance_installer
                .as_ref()
                .unwrap_or(&current)
                .display()
                .to_string();
            for (name, value) in [
                ("DisplayName", "Beatblock Together".to_string()),
                ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
                ("Publisher", "Beatblock Together".to_string()),
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
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\BeatblockTogether",
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
            .find(|path| validate_game_directory(path).is_ok())
    }

    fn manifest_path(&self) -> PathBuf {
        self.data_dir.join("install-manifest.json")
    }

    fn load_manifest(&self) -> Result<Option<InstallManifest>> {
        let path = self.manifest_path();
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(path)?;
        // Windows maintenance tools sometimes rewrite JSON with a UTF-8 BOM.
        // Accept it as a legacy migration input; all new writes remain BOM-free.
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        Ok(Some(serde_json::from_slice(bytes)?))
    }

    fn save_manifest(&self, manifest: &InstallManifest) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        atomic_write(&self.manifest_path(), &serde_json::to_vec_pretty(manifest)?)?;
        Ok(())
    }
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

fn short_hash(value: &str) -> String {
    if value.len() > 16 {
        format!("{}…", &value[..16])
    } else {
        value.into()
    }
}

fn file_matches(path: &Path, expected: &[u8]) -> bool {
    std::fs::read(path)
        .ok()
        .is_some_and(|bytes| Sha256::digest(&bytes)[..] == Sha256::digest(expected)[..])
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
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&temporary, path)?;
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
            return Err(error).context("activate staged Beatblock Together mod");
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

#[cfg(windows)]
fn quote_windows(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn json_paths(plugin: &Path, locale: &Path) -> Vec<PathBuf> {
    vec![plugin.to_owned(), locale.to_owned()]
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
        "name=Beatblock Together Host",
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
            ProjectDirs::from("org", "BeatblockTogether", "BeatblockTogether")
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
        entry.file_name() != "BeatblockTogether" && entry.path().join("lovely").is_dir()
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn supported_game_hash() -> &'static str {
    "c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b"
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

    #[test]
    fn detector_accepts_reference_game_shape() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.reference/Beatblock");
        assert!(validate_game_directory(&root).is_ok());
        assert_eq!(
            sha256_file(&root.join("Beatblock.exe")).unwrap(),
            supported_game_hash()
        );
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
    fn arbitrary_unicode_game_folder_is_valid_but_uncertified() {
        let root = std::env::temp_dir().join(format!("BBT Player 测试 {}", rand::random::<u64>()));
        let game = root.join("My Beatblock Copy");
        fake_game(&game);
        let installer = Installer::with_mods_directory(root.join("data"), root.join("mods"));
        let inspection = installer.inspect_target(&game);
        assert!(inspection.valid);
        assert!(!inspection.supported_build);
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
        let mod_dir = mods.join("BeatblockTogether");
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
        let runtime = data.join("runtime/BeatblockTogetherRuntime.exe");
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
    fn uninstall_preserves_data_unless_explicitly_requested() {
        let root = std::env::temp_dir().join(format!("bbt-uninstall-{}", rand::random::<u64>()));
        let data = root.join("data");
        let mods = root.join("mods");
        let game = root.join("game");
        std::fs::create_dir_all(mods.join("BeatblockTogether")).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("runtime.sqlite3"), b"history").unwrap();
        let installer = Installer::new(data.clone());
        let manifest = InstallManifest {
            version: "test".into(),
            game_directory: game.clone(),
            mods_directory: mods.clone(),
            distribution: Distribution::Standalone,
            installed_files: Vec::new(),
            lovely_owned: false,
            lovely_backup: None,
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

    #[cfg(windows)]
    #[test]
    fn firewall_command_targets_runtime_and_selected_profiles() {
        let runtime = Path::new(r"C:\Program Files\Beatblock Together/Runtime.exe");
        let private = firewall_command(runtime, false, true)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            private.contains(&r"program=C:\Program Files\Beatblock Together\Runtime.exe".into())
        );
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
}
