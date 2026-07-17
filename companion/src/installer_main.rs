#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::Result;
use beatblock_together_companion::{
    gui,
    installer::{
        write_operation_status, Distribution, Installer, OperationKind, OperationProgress, Severity,
    },
};
use clap::Parser;
use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about = "Install and maintain Beatblock Together")]
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
    let args = Args::parse();
    let operation_file = args.operation_file.clone();
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
    let data_dir = args.data_dir.unwrap_or_else(|| {
        ProjectDirs::from("org", "BeatblockTogether", "BeatblockTogether")
            .map(|dirs| dirs.data_local_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("installer-data"))
    });
    std::fs::create_dir_all(&data_dir)?;
    let installer = Installer::new(data_dir.clone());
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
