#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{bail, Context, Result};
use beatblock_online_companion::{
    gui,
    installer::{
        write_operation_status, Distribution, Installer, OperationKind, OperationProgress, Severity,
    },
};
use clap::Parser;
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(version, about = "Install and maintain Beatblock Online")]
struct Args {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    game_dir: Option<PathBuf>,
    #[arg(long)]
    allow_unknown_build: bool,
    #[arg(long, value_parser = ["automatic", "standalone", "beatblock-plus"], default_value = "automatic")]
    method: String,
    #[arg(long)]
    install_now: bool,
    #[arg(long)]
    install_obs: bool,
    /// Explicit OBS root selected by the visible installer. This is carried
    /// across UAC so portable/custom installations do not depend on inherited
    /// environment variables.
    #[arg(long)]
    obs_dir: Option<PathBuf>,
    #[arg(long)]
    firewall_public: bool,
    #[arg(long)]
    repair_now: bool,
    #[arg(long)]
    uninstall_now: bool,
    #[arg(long)]
    restore_now: bool,
    #[arg(long)]
    remove_user_data: bool,
    /// Atomic progress handoff used by the visible unelevated installer.
    #[arg(long)]
    operation_file: Option<PathBuf>,
}

fn main() {
    let mut args = Args::parse();
    let data_dir = resolved_data_dir(args.data_dir.as_ref());
    let operation_file = match validate_operation_file(&data_dir, args.operation_file.as_deref()) {
        Ok(path) => path,
        Err(_) => std::process::exit(1),
    };
    // Keep only the path that passed the ownership and symlink checks. Error
    // reporting must never reuse an untrusted command-line destination.
    args.operation_file = operation_file.clone();
    let operation = requested_operation(&args);
    if let Err(error) = run(args) {
        // The GUI-subsystem helper has no terminal. Publish the full anyhow
        // chain so the visible installer can explain the privileged failure.
        if let Some(path) = operation_file.as_deref() {
            let _ = write_operation_status(
                path,
                &OperationProgress {
                    operation,
                    phase: "failed".into(),
                    percent: 100,
                    message: format!("{error:#}"),
                    severity: Severity::Error,
                    terminal: true,
                },
            );
        }
        std::process::exit(1);
    }
}

fn requested_operation(args: &Args) -> OperationKind {
    if args.repair_now {
        OperationKind::Repair
    } else if args.uninstall_now {
        OperationKind::Uninstall
    } else if args.restore_now {
        OperationKind::Restore
    } else {
        OperationKind::Install
    }
}

fn run(args: Args) -> Result<()> {
    let requested = [
        args.install_now,
        args.repair_now,
        args.uninstall_now,
        args.restore_now,
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();
    if requested > 1 {
        bail!("choose exactly one maintenance operation");
    }
    let data_dir = resolved_data_dir(args.data_dir.as_ref());
    std::fs::create_dir_all(&data_dir)?;
    let installer = Installer::new(data_dir.clone());
    installer.set_obs_directory(args.obs_dir.clone())?;
    if args.install_now {
        let distribution = match args.method.as_str() {
            "standalone" => Some(Distribution::Standalone),
            "beatblock-plus" => Some(Distribution::BeatblockPlus),
            _ => None,
        };
        let operation_file = args.operation_file.clone();
        let publish = |event: OperationProgress| {
            if !event.terminal {
                if let Some(path) = operation_file.as_deref() {
                    let _ = write_operation_status(path, &event);
                }
            }
        };
        installer.install_with_optional_obs(
            args.game_dir,
            args.allow_unknown_build,
            distribution,
            args.firewall_public,
            args.install_obs,
            publish,
        )?;
        if let Some(path) = args.operation_file.as_deref() {
            write_operation_status(
                path,
                &OperationProgress {
                    operation: OperationKind::Install,
                    phase: "complete".into(),
                    percent: 100,
                    message: "Administrator installation completed".into(),
                    severity: Severity::Success,
                    terminal: true,
                },
            )?;
        }
        return Ok(());
    }
    if args.repair_now {
        let file = args.operation_file.clone();
        installer.repair_with_progress(|event| {
            if let Some(path) = file.as_deref() {
                let _ = write_operation_status(path, &event);
            }
        })?;
        return Ok(());
    }
    if args.uninstall_now {
        let file = args.operation_file.clone();
        installer.uninstall_with_progress(args.remove_user_data, |event| {
            if let Some(path) = file.as_deref() {
                let _ = write_operation_status(path, &event);
            }
        })?;
        return Ok(());
    }
    if args.restore_now {
        let file = args.operation_file.clone();
        installer.restore_with_progress(|event| {
            if let Some(path) = file.as_deref() {
                let _ = write_operation_status(path, &event);
            }
        })?;
        return Ok(());
    }
    gui::run(data_dir)
}

fn resolved_data_dir(explicit: Option<&PathBuf>) -> PathBuf {
    explicit.cloned().unwrap_or_else(|| {
        ProjectDirs::from("org", "BeatblockOnline", "BeatblockOnline")
            .map(|dirs| dirs.data_local_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("installer-data"))
    })
}

fn validate_operation_file(data_dir: &Path, requested: Option<&Path>) -> Result<Option<PathBuf>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let operations = data_dir.join("operations");
    std::fs::create_dir_all(&operations).context("create operation-status directory")?;
    if std::fs::symlink_metadata(&operations)?
        .file_type()
        .is_symlink()
    {
        bail!("operation-status directory cannot be a symlink");
    }
    let expected_parent = std::fs::canonicalize(&operations)?;
    let parent = requested
        .parent()
        .context("operation-status path has no parent")?;
    if std::fs::canonicalize(parent)? != expected_parent {
        bail!("operation-status path is outside the managed directory");
    }
    if requested
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("json"))
    {
        bail!("operation-status file must use a .json extension");
    }
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .context("operation-status filename is not valid UTF-8")?;
    uuid::Uuid::parse_str(stem).context("operation-status filename must be a UUID")?;
    if std::fs::symlink_metadata(requested).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        bail!("operation-status file cannot be a symlink");
    }
    Ok(Some(requested.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_status_is_confined_to_uuid_files_in_managed_directory() {
        let root =
            std::env::temp_dir().join(format!("bbt-operation-path-{}", uuid::Uuid::new_v4()));
        let operations = root.join("operations");
        std::fs::create_dir_all(&operations).unwrap();
        let valid = operations.join(format!("{}.json", uuid::Uuid::new_v4()));
        assert_eq!(
            validate_operation_file(&root, Some(&valid)).unwrap(),
            Some(valid)
        );
        assert!(validate_operation_file(&root, Some(&root.join("victim.json"))).is_err());
        assert!(validate_operation_file(&root, Some(&operations.join("not-a-uuid.json"))).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_line_accepts_an_explicit_obs_directory() {
        let args = Args::try_parse_from([
            "installer",
            "--install-now",
            "--install-obs",
            "--obs-dir",
            r"D:\Portable Apps\obs-studio",
        ])
        .unwrap();
        assert_eq!(
            args.obs_dir,
            Some(PathBuf::from(r"D:\Portable Apps\obs-studio"))
        );
    }
}
