//! Installer-only Slint UI. Runtime/room controls intentionally do not exist in
//! this binary; every online operation remains inside Beatblock.

use crate::installer::{
    distribution_label, ComponentState, Distribution, Installer, OperationKind, OperationProgress,
    TargetInspection,
};
use anyhow::Result;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

slint::slint! {
    import { Button, CheckBox, ComboBox, LineEdit, ProgressIndicator, ScrollView } from "std-widgets.slint";

    export struct ComponentRow {
        name: string,
        state: string,
        included: string,
        details: string,
        badge-color: color,
        badge-text: color,
    }

    component StatusBadge inherits Rectangle {
        in property <string> label;
        in property <color> badge-color;
        in property <color> text-color;
        height: 25px;
        min-width: 104px;
        border-radius: 4px;
        background: badge-color;
        Text { text: label; color: text-color; font-size: 11px; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; }
    }

    export component InstallerWindow inherits Window {
        title: "Beatblock Together Installer";
        width: 820px;
        height: 690px;
        background: #edf1f4;

        in-out property<int> page: 0;
        in-out property<string> game-path: "";
        in-out property<string> selected-path-caption: "No folder selected";
        in-out property<string> build-label: "SCANNING";
        in-out property<color> build-color: #dce8f4;
        in-out property<color> build-text: #24577d;
        in-out property<string> method-label: "AUTOMATIC";
        in-out property<color> method-color: #dce8f4;
        in-out property<color> method-text: #24577d;
        in-out property<string> install-state: "NOT INSTALLED";
        in-out property<color> state-color: #e2e6ea;
        in-out property<color> state-text: #515b64;
        in-out property<string> target-detail: "Choose the folder containing Beatblock.exe.";
        in-out property<string> primary-label: "Install / Update";
        in-out property<bool> repairable-components: false;
        in-out property<[ComponentRow]> components;
        in-out property<string> log-text: "Beatblock Together installer started.\n";
        in-out property<bool> install-obs: false;
        in-out property<bool> firewall-public: false;
        in-out property<bool> remove-user-data: false;
        in-out property<bool> allow-unknown-build: false;
        in-out property<int> install-method: 0;
        in-out property<bool> busy: false;
        in-out property<bool> scanning: false;
        in-out property<float> operation-progress: 0;
        in-out property<string> operation-step: "Ready to install or maintain Beatblock Together.";
        in-out property<string> operation-percent: "";
        in-out property<bool> result-visible: false;
        in-out property<string> result-text: "";
        in-out property<color> result-background: #dcefe2;
        in-out property<color> result-foreground: #1d6337;
        in-out property<bool> dialog-visible: false;
        in-out property<bool> dialog-confirmation: false;
        in-out property<bool> dialog-can-launch: false;
        in-out property<string> dialog-title: "Completed";
        in-out property<string> dialog-body: "";
        in-out property<string> confirmation-kind: "";

        callback browse();
        callback path-edited(string);
        callback install();
        callback repair();
        callback uninstall();
        callback restore();
        callback launch();
        callback confirm-action(string);
        callback refresh();
        callback save-log();
        callback copy-log();
        callback open-backup();
        callback check-update();

        VerticalLayout {
            spacing: 0px;
            Rectangle {
                height: 62px; background: #ffffff; border-width: 1px; border-color: #cbd2d9;
                HorizontalLayout {
                    padding-left: 22px; padding-right: 22px;
                    VerticalLayout { alignment: center; spacing: 1px;
                        Text { text: "Beatblock Together"; font-size: 22px; font-weight: 700; color: #18222c; }
                        Text { text: "Installer & maintenance"; font-size: 11px; color: #66727d; }
                    }
                    Rectangle { horizontal-stretch: 1; background: transparent; }
                    StatusBadge { label: root.busy ? "WORKING" : "READY"; badge-color: root.busy ? #dce8f4 : #dcefe2; text-color: root.busy ? #24577d : #1d6337; }
                }
            }
            Rectangle {
                height: 47px; background: #e2e7eb; border-width: 1px; border-color: #cbd2d9;
                HorizontalLayout {
                    padding-left: 12px; padding-right: 12px; spacing: 5px;
                    Button { text: "Install"; enabled: !root.busy; clicked => { root.page = 0; } }
                    Button { text: "Components"; enabled: !root.busy; clicked => { root.page = 1; root.refresh(); } }
                    Button { text: "Log"; clicked => { root.page = 2; } }
                    Button { text: "Settings"; enabled: !root.busy; clicked => { root.page = 3; } }
                    Rectangle { horizontal-stretch: 1; background: transparent; }
                }
            }
            Rectangle {
                background: #f6f8fa;
                if root.page == 0: ScrollView {
                    VerticalLayout {
                        padding: 20px; spacing: 13px;
                        Text { text: "Selected game"; color: #18222c; font-size: 15px; font-weight: 700; }
                        Rectangle {
                            height: 202px; background: #ffffff; border-width: 1px; border-color: #cbd2d9; border-radius: 5px;
                            VerticalLayout { padding: 14px; spacing: 10px;
                                Text { text: root.selected-path-caption; color: #28333d; font-size: 11px; overflow: elide; }
                                HorizontalLayout { spacing: 8px;
                                    LineEdit { text <=> root.game-path; enabled: !root.busy; horizontal-stretch: 1; edited(value) => { root.path-edited(value); } }
                                    Button { text: "Browse…"; width: 88px; enabled: !root.busy; clicked => { root.browse(); } }
                                }
                                HorizontalLayout { spacing: 10px;
                                    Text { text: "Build"; color: #5a6570; font-size: 12px; width: 75px; vertical-alignment: center; }
                                    StatusBadge { label: root.build-label; badge-color: root.build-color; text-color: root.build-text; }
                                    Rectangle { horizontal-stretch: 1; background: transparent; }
                                }
                                HorizontalLayout { spacing: 10px;
                                    Text { text: "Method"; color: #5a6570; font-size: 12px; width: 75px; vertical-alignment: center; }
                                    StatusBadge { label: root.method-label; badge-color: root.method-color; text-color: root.method-text; }
                                    Rectangle { horizontal-stretch: 1; background: transparent; }
                                }
                                HorizontalLayout { spacing: 10px;
                                    Text { text: "State"; color: #5a6570; font-size: 12px; width: 75px; vertical-alignment: center; }
                                    StatusBadge { label: root.install-state; badge-color: root.state-color; text-color: root.state-text; }
                                    Rectangle { horizontal-stretch: 1; background: transparent; }
                                }
                                Text { text: root.target-detail; color: #596570; font-size: 11px; wrap: word-wrap; }
                            }
                        }
                        HorizontalLayout { spacing: 10px;
                            VerticalLayout { spacing: 5px;
                                Text { text: "Installation method"; color: #46515b; font-size: 12px; }
                                ComboBox { enabled: !root.busy; model: ["Automatic (recommended)", "Standalone Lovely", "BeatblockPlus 2.x"]; current-index <=> root.install-method; }
                            }
                            VerticalLayout { spacing: 5px;
                                Text { text: "Optional integration"; color: #46515b; font-size: 12px; }
                                CheckBox { enabled: !root.busy; text: "Install OBS source when detected"; checked <=> root.install-obs; }
                            }
                        }
                        Rectangle {
                            height: root.busy ? 67px : 0px; visible: root.busy; background: #eef4fa; border-width: 1px; border-color: #b9cee1; border-radius: 4px;
                            VerticalLayout { padding: 10px; spacing: 6px;
                                HorizontalLayout { Text { text: root.operation-step; color: #29465f; font-size: 11px; } Rectangle { horizontal-stretch: 1; background: transparent; } Text { text: root.operation-percent; color: #29465f; font-size: 11px; font-weight: 700; } }
                                ProgressIndicator { progress: root.operation-progress; indeterminate: root.scanning; }
                            }
                        }
                        Button { text: root.primary-label; height: 46px; enabled: !root.busy && root.install-state != "INVALID TARGET"; clicked => { root.install(); } }
                        HorizontalLayout { spacing: 8px;
                            Button { text: root.install-state == "READY" ? "Launch Beatblock" : "Repair"; enabled: !root.busy && (root.install-state == "READY" || root.install-state == "REPAIR REQUIRED"); clicked => { if root.install-state == "READY" { root.launch(); } else { root.repair(); } } }
                            Button { text: "Uninstall"; enabled: !root.busy && (root.install-state == "READY" || root.install-state == "REPAIR REQUIRED"); clicked => { root.confirmation-kind = "uninstall"; root.dialog-confirmation = true; root.dialog-title = "Uninstall Beatblock Together?"; root.dialog-body = "The managed mod, runtime, firewall rule, and optional OBS source will be removed. Settings and history follow your Settings choice."; root.dialog-visible = true; } }
                            Button { text: "Restore game files"; enabled: !root.busy && (root.install-state == "READY" || root.install-state == "REPAIR REQUIRED"); clicked => { root.confirmation-kind = "restore"; root.dialog-confirmation = true; root.dialog-title = "Restore game files?"; root.dialog-body = "The BBT mod is removed and the preserved injector state is restored. You can reinstall later."; root.dialog-visible = true; } }
                        }
                    }
                }
                if root.page == 1: ScrollView {
                    VerticalLayout { padding: 20px; spacing: 10px;
                        HorizontalLayout { Text { text: "Installed components"; color: #18222c; font-size: 18px; font-weight: 700; } Rectangle { horizontal-stretch: 1; background: transparent; } Button { text: "Refresh"; enabled: !root.busy; clicked => { root.refresh(); } } }
                        Rectangle { height: 34px; background: #dfe5ea; border-radius: 4px;
                            HorizontalLayout { padding-left: 12px; padding-right: 12px; spacing: 10px;
                                Text { text: "Component"; width: 170px; color: #35414c; font-size: 11px; font-weight: 700; vertical-alignment: center; }
                                Text { text: "Current state"; width: 120px; color: #35414c; font-size: 11px; font-weight: 700; vertical-alignment: center; }
                                Text { text: "Included"; width: 75px; color: #35414c; font-size: 11px; font-weight: 700; vertical-alignment: center; }
                                Text { text: "Details"; color: #35414c; font-size: 11px; font-weight: 700; vertical-alignment: center; horizontal-stretch: 1; }
                            }
                        }
                        for row in root.components: Rectangle {
                            height: 43px; background: #ffffff; border-width: 1px; border-color: #d6dce1; border-radius: 3px;
                            HorizontalLayout { padding-left: 12px; padding-right: 12px; spacing: 10px;
                                Text { text: row.name; width: 170px; color: #28333d; font-size: 12px; vertical-alignment: center; }
                                StatusBadge { width: 120px; label: row.state; badge-color: row.badge-color; text-color: row.badge-text; }
                                Text { text: row.included; width: 75px; color: #596570; font-size: 11px; vertical-alignment: center; }
                                Text { text: row.details; color: #596570; font-size: 11px; vertical-alignment: center; overflow: elide; horizontal-stretch: 1; }
                            }
                        }
                        Button { text: "Repair Required Components"; height: 42px; enabled: !root.busy && root.repairable-components; clicked => { root.repair(); } }
                    }
                }
                if root.page == 2: VerticalLayout { padding: 20px; spacing: 10px;
                    HorizontalLayout { Text { text: "Installation log"; color: #18222c; font-size: 18px; font-weight: 700; } Rectangle { horizontal-stretch: 1; background: transparent; } }
                    Rectangle { vertical-stretch: 1; background: #151b21; border-radius: 4px;
                        ScrollView { Text { x: 12px; y: 10px; width: parent.width - 24px; text: root.log-text; color: #dbe3e9; font-size: 11px; wrap: word-wrap; } }
                    }
                    HorizontalLayout { spacing: 8px; Button { text: "Copy"; clicked => { root.copy-log(); } } Button { text: "Save log…"; clicked => { root.save-log(); } } Rectangle { horizontal-stretch: 1; background: transparent; } }
                }
                if root.page == 3: ScrollView { VerticalLayout { padding: 20px; spacing: 14px;
                    Text { text: "Installer settings"; color: #18222c; font-size: 18px; font-weight: 700; }
                    CheckBox { enabled: !root.busy; text: "Allow hosting on Public Windows Firewall profiles"; checked <=> root.firewall-public; }
                    CheckBox { enabled: !root.busy; text: "Remove settings and match history during Uninstall"; checked <=> root.remove-user-data; }
                    CheckBox { enabled: !root.busy; text: "Developer: allow an uncertified Beatblock build"; checked <=> root.allow-unknown-build; }
                    Rectangle { height: 105px; background: #ffffff; border-width: 1px; border-color: #cbd2d9; border-radius: 4px;
                        VerticalLayout { padding: 13px; spacing: 6px; Text { text: "Stable update channel"; color: #35414c; font-size: 13px; font-weight: 700; } Text { text: "Update checks run only while this installer is open and always require confirmation."; color: #596570; font-size: 11px; } Button { text: "Check for updates"; enabled: !root.busy; clicked => { root.check-update(); } } }
                    }
                    Button { text: "Open backup folder"; clicked => { root.open-backup(); } }
                } }
            }
            if root.result-visible: Rectangle {
                height: 44px; background: root.result-background; border-width: 1px; border-color: root.result-foreground;
                HorizontalLayout { padding-left: 14px; padding-right: 14px; spacing: 10px; Text { text: root.result-text; color: root.result-foreground; font-size: 12px; font-weight: 600; vertical-alignment: center; } Rectangle { horizontal-stretch: 1; background: transparent; } }
            }
        }

        if root.dialog-visible: Rectangle {
            x: 0px; y: 0px; width: root.width; height: root.height; background: #00000088;
            TouchArea { width: parent.width; height: parent.height; }
            Rectangle {
                width: 470px; height: 235px; x: (parent.width - self.width) / 2; y: (parent.height - self.height) / 2;
                background: #ffffff; border-radius: 7px; border-width: 1px; border-color: #9da8b2;
                VerticalLayout { padding: 22px; spacing: 14px;
                    Text { text: root.dialog-title; color: #18222c; font-size: 20px; font-weight: 700; }
                    Text { text: root.dialog-body; color: #4f5c67; font-size: 12px; wrap: word-wrap; vertical-stretch: 1; }
                    HorizontalLayout { spacing: 8px;
                        if root.dialog-can-launch && !root.dialog-confirmation: Button { text: "Launch Beatblock"; clicked => { root.dialog-visible = false; root.launch(); } }
                        if !root.dialog-confirmation: Button { text: "View Components"; clicked => { root.page = 1; root.dialog-visible = false; root.refresh(); } }
                        if root.dialog-confirmation: Button { text: "Confirm"; clicked => { root.dialog-visible = false; root.confirm-action(root.confirmation-kind); } }
                        Rectangle { horizontal-stretch: 1; background: transparent; }
                        Button { text: root.dialog-confirmation ? "Cancel" : "Close"; clicked => { root.dialog-visible = false; root.dialog-confirmation = false; } }
                    }
                }
            }
        }
    }
}

fn colors(state: ComponentState) -> (slint::Color, slint::Color) {
    match state {
        ComponentState::Ready => (
            slint::Color::from_rgb_u8(220, 239, 226),
            slint::Color::from_rgb_u8(29, 99, 55),
        ),
        ComponentState::Attention | ComponentState::Optional => (
            slint::Color::from_rgb_u8(255, 239, 194),
            slint::Color::from_rgb_u8(117, 77, 5),
        ),
        ComponentState::Missing | ComponentState::Broken => (
            slint::Color::from_rgb_u8(250, 220, 220),
            slint::Color::from_rgb_u8(139, 37, 37),
        ),
        ComponentState::NotInstalled => (
            slint::Color::from_rgb_u8(226, 230, 234),
            slint::Color::from_rgb_u8(81, 91, 100),
        ),
    }
}

fn inspection_rows(inspection: &TargetInspection) -> ModelRc<ComponentRow> {
    let rows = inspection
        .components
        .iter()
        .map(|item| {
            let (badge_color, badge_text) = colors(item.state);
            ComponentRow {
                name: item.name.clone().into(),
                state: item.label.clone().into(),
                included: item.included.clone().into(),
                details: item.details.clone().into(),
                badge_color,
                badge_text,
            }
        })
        .collect::<Vec<_>>();
    ModelRc::new(VecModel::from(rows))
}

fn set_badge(window: &InstallerWindow, inspection: &TargetInspection) {
    window.set_selected_path_caption(inspection.game_directory.display().to_string().into());
    let (build_bg, build_fg) = colors(if !inspection.valid {
        ComponentState::Broken
    } else if inspection.supported_build {
        ComponentState::Ready
    } else {
        ComponentState::Attention
    });
    window.set_build_label(
        if !inspection.valid {
            "INVALID"
        } else if inspection.supported_build {
            "SUPPORTED"
        } else {
            "UNCERTIFIED"
        }
        .into(),
    );
    window.set_build_color(build_bg);
    window.set_build_text(build_fg);
    let (method_bg, method_fg) = colors(ComponentState::Ready);
    window.set_method_label(
        distribution_label(inspection.distribution)
            .to_ascii_uppercase()
            .into(),
    );
    window.set_method_color(method_bg);
    window.set_method_text(method_fg);
    let state_kind = match inspection.install_state.as_str() {
        "READY" => ComponentState::Ready,
        "REPAIR REQUIRED" | "MOVE INSTALLATION" => ComponentState::Attention,
        "INVALID TARGET" => ComponentState::Broken,
        _ => ComponentState::NotInstalled,
    };
    let (state_bg, state_fg) = colors(state_kind);
    window.set_install_state(inspection.install_state.clone().into());
    window.set_state_color(state_bg);
    window.set_state_text(state_fg);
    window.set_primary_label(
        if inspection.managed_elsewhere.is_some() {
            "Move Installation"
        } else {
            "Install / Update"
        }
        .into(),
    );
    window.set_target_detail(inspection.message.clone().into());
    window.set_components(inspection_rows(inspection));
    window.set_repairable_components(inspection.components.iter().any(|component| {
        component.included == "Yes"
            && matches!(
                component.state,
                ComponentState::Attention | ComponentState::Missing | ComponentState::Broken
            )
    }));
}

fn append_log(window: &InstallerWindow, message: &str) {
    let mut log = window.get_log_text().to_string();
    if log.len() > 48_000 {
        log = log.split_off(log.len() - 32_000);
    }
    log.push_str(message);
    log.push('\n');
    window.set_log_text(log.into());
}

fn post_progress(weak: slint::Weak<InstallerWindow>, event: OperationProgress) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_busy(!event.terminal);
            window.set_scanning(event.operation == OperationKind::Inspect);
            window.set_operation_progress(event.percent as f32 / 100.0);
            window.set_operation_percent(format!("{}%", event.percent).into());
            window.set_operation_step(event.message.clone().into());
            append_log(&window, &format!("[{}%] {}", event.percent, event.message));
        }
    });
}

fn terminal(
    weak: slint::Weak<InstallerWindow>,
    title: &str,
    result: Result<String>,
    can_launch: bool,
) {
    let title = title.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = weak.upgrade() {
            window.set_busy(false);
            window.set_scanning(false);
            window.set_operation_progress(1.0);
            window.set_operation_percent("100%".into());
            match result {
                Ok(message) => {
                    window.set_result_background(slint::Color::from_rgb_u8(220, 239, 226));
                    window.set_result_foreground(slint::Color::from_rgb_u8(29, 99, 55));
                    window.set_result_text(message.clone().into());
                    window.set_dialog_title(title.into());
                    window.set_dialog_body(message.clone().into());
                    window.set_dialog_can_launch(can_launch);
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    window.set_result_background(slint::Color::from_rgb_u8(250, 220, 220));
                    window.set_result_foreground(slint::Color::from_rgb_u8(139, 37, 37));
                    let failed_title = title.strip_suffix(" complete").unwrap_or(&title);
                    window.set_result_text(format!("Operation failed: {message}").into());
                    window.set_dialog_title(format!("{failed_title} failed").into());
                    window.set_dialog_body(message.clone().into());
                    window.set_dialog_can_launch(false);
                    append_log(&window, &format!("ERROR: {message}"));
                }
            }
            window.set_result_visible(true);
            window.set_dialog_confirmation(false);
            window.set_dialog_visible(true);
            window.invoke_refresh();
        }
    });
}

fn refresh_selected(window: &InstallerWindow, installer: &Installer) {
    let path = PathBuf::from(window.get_game_path().as_str());
    if path.as_os_str().is_empty() {
        return;
    }
    set_badge(window, &installer.inspect_target(&path));
}

fn selected_options(window: &InstallerWindow) -> (PathBuf, bool, Option<Distribution>, bool, bool) {
    let distribution = match window.get_install_method() {
        1 => Some(Distribution::Standalone),
        2 => Some(Distribution::BeatblockPlus),
        _ => None,
    };
    (
        PathBuf::from(window.get_game_path().as_str()),
        window.get_allow_unknown_build(),
        distribution,
        window.get_install_obs(),
        window.get_firewall_public(),
    )
}

fn begin_install(
    window: &InstallerWindow,
    weak: slint::Weak<InstallerWindow>,
    installer: Arc<Installer>,
    data_dir: PathBuf,
) {
    let (path, allow_unknown, distribution, install_obs, firewall_public) =
        selected_options(window);
    window.set_busy(true);
    window.set_result_visible(false);
    window.set_operation_step("Starting installation…".into());
    window.set_operation_progress(0.0);
    std::thread::spawn(move || {
        let run = || -> Result<String> {
            let mut installed_elevated = false;
            let mut progress = |event: OperationProgress| {
                if !event.terminal {
                    post_progress(weak.clone(), event);
                }
            };
            match installer.install_with_progress_options(
                Some(path.clone()),
                allow_unknown,
                distribution,
                firewall_public,
                &mut progress,
            ) {
                Ok(_) => {}
                Err(error) if needs_elevation(&error) => {
                    let operation_file =
                        data_dir.join(format!("operations/{}.json", uuid::Uuid::new_v4()));
                    if let Some(parent) = operation_file.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let arguments = elevated_install_arguments(
                        &data_dir,
                        Some(&path),
                        allow_unknown,
                        distribution,
                        install_obs,
                        firewall_public,
                    );
                    Installer::request_elevated_with_progress(
                        &arguments,
                        &operation_file,
                        |event| {
                            if !event.terminal {
                                post_progress(weak.clone(), event)
                            }
                        },
                    )?;
                    installed_elevated = true;
                }
                Err(error) => return Err(error),
            }
            if install_obs && !installed_elevated {
                installer.install_obs_plugin()?;
            }
            let inspection = installer.inspect_target(&path);
            if inspection.repair_required {
                anyhow::bail!(
                    "post-install verification found required components that still need repair"
                );
            }
            Ok(format!(
                "Beatblock Together is ready in {}.",
                path.display()
            ))
        }();
        terminal(weak, "Installation complete", run, true);
    });
}

pub fn run(data_dir: PathBuf) -> Result<()> {
    let window = InstallerWindow::new()?;
    let installer = Arc::new(Installer::new(data_dir.clone()));
    if let Some(path) = installer.initial_game_directory() {
        window.set_game_path(path.display().to_string().into());
        refresh_selected(&window, &installer);
    }

    {
        let weak = window.as_weak();
        let installer = installer.clone();
        window.on_browse(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose the folder containing Beatblock.exe")
                .pick_folder()
            {
                if let Some(window) = weak.upgrade() {
                    window.set_game_path(path.display().to_string().into());
                    refresh_selected(&window, &installer);
                }
            }
        });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        window.on_path_edited(move |value| {
            if let Some(window) = weak.upgrade() {
                if Path::new(value.as_str()).is_dir() {
                    refresh_selected(&window, &installer);
                }
            }
        });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        let data = data_dir.clone();
        window.on_install(move || if let Some(window) = weak.upgrade() { let inspection = installer.inspect_target(Path::new(window.get_game_path().as_str())); if inspection.managed_elsewhere.is_some() { window.set_confirmation_kind("move".into()); window.set_dialog_confirmation(true); window.set_dialog_title("Move the managed installation?".into()); window.set_dialog_body(format!("The injector in {} will be restored before BBT is installed into the selected folder.", inspection.managed_elsewhere.unwrap().display()).into()); window.set_dialog_visible(true); } else { begin_install(&window, weak.clone(), installer.clone(), data.clone()); } });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        let data = data_dir.clone();
        window.on_repair(move || {
            if let Some(window) = weak.upgrade() {
                window.set_busy(true);
                window.set_result_visible(false);
            }
            let w = weak.clone();
            let i = installer.clone();
            let data = data.clone();
            std::thread::spawn(move || {
                let first = i.repair_with_progress(|event| post_progress(w.clone(), event));
                let result = match first {
                    Ok(m) => Ok(format!(
                        "Required components are repaired in {}.",
                        m.game_directory.display()
                    )),
                    Err(error) if needs_elevation(&error) => {
                        elevate_simple(&data, "--repair-now", false, w.clone()).map(|_| {
                            "Required components were repaired with administrator access.".into()
                        })
                    }
                    Err(error) => Err(error),
                };
                terminal(w, "Repair complete", result, true);
            });
        });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        let data = data_dir.clone();
        window.on_confirm_action(move |kind| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            match kind.as_str() {
                "move" => begin_install(&window, weak.clone(), installer.clone(), data.clone()),
                "uninstall" => {
                    let remove = window.get_remove_user_data();
                    window.set_busy(true);
                    let w = weak.clone();
                    let i = installer.clone();
                    let data = data.clone();
                    std::thread::spawn(move || {
                        let first =
                            i.uninstall_with_progress(remove, |e| post_progress(w.clone(), e));
                        let r = match first {
                            Ok(()) => Ok("Beatblock Together was removed.".into()),
                            Err(error) if needs_elevation(&error) => {
                                elevate_simple(&data, "--uninstall-now", remove, w.clone()).map(
                                    |_| {
                                        "Beatblock Together was removed with administrator access."
                                            .into()
                                    },
                                )
                            }
                            Err(error) => Err(error),
                        };
                        terminal(w, "Uninstall complete", r, false);
                    });
                }
                "restore" => {
                    window.set_busy(true);
                    let w = weak.clone();
                    let i = installer.clone();
                    let data = data.clone();
                    std::thread::spawn(move || {
                        let first = i.restore_with_progress(|e| post_progress(w.clone(), e));
                        let r = match first {
                            Ok(()) => {
                                Ok("The managed mod was removed and game files were restored."
                                    .into())
                            }
                            Err(error) if needs_elevation(&error) => {
                                elevate_simple(&data, "--restore-now", false, w.clone()).map(|_| {
                                    "Game files were restored with administrator access.".into()
                                })
                            }
                            Err(error) => Err(error),
                        };
                        terminal(w, "Restore complete", r, false);
                    });
                }
                _ => {}
            }
        });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        window.on_refresh(move || {
            if let Some(window) = weak.upgrade() {
                refresh_selected(&window, &installer);
            }
        });
    }
    {
        let weak = window.as_weak();
        let installer = installer.clone();
        window.on_launch(move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let path = PathBuf::from(window.get_game_path().as_str());
            window.set_busy(true);
            window.set_dialog_visible(false);
            let w = weak.clone();
            let i = installer.clone();
            std::thread::spawn(move || {
                let r = i
                    .launch_and_verify(&path, |e| post_progress(w.clone(), e))
                    .map(|report| {
                        format!(
                            "{}\nLovely log: {}",
                            report.message,
                            report.log_path.display()
                        )
                    });
                terminal(w, "Launch verification complete", r, false);
            });
        });
    }
    {
        let weak = window.as_weak();
        window.on_copy_log(move || {
            if let Some(window) = weak.upgrade() {
                if let Err(error) = copy_to_clipboard(&window.get_log_text()) {
                    append_log(&window, &format!("Copy failed: {error:#}"));
                }
            }
        });
    }
    {
        let weak = window.as_weak();
        window.on_save_log(move || {
            if let Some(window) = weak.upgrade() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("BeatblockTogether-install.log")
                    .save_file()
                {
                    let _ = std::fs::write(path, window.get_log_text().as_bytes());
                }
            }
        });
    }
    {
        let backup = data_dir.join("backups");
        window.on_open_backup(move || {
            let _ = open::that(&backup);
        });
    }
    {
        let weak = window.as_weak();
        window.on_check_update(move || if let Some(window)=weak.upgrade(){ window.set_result_visible(true); window.set_result_text("You are on the stable 0.3 alpha channel. No signed update manifest is configured for this development build.".into()); });
    }
    {
        let weak = window.as_weak();
        window.window().on_close_requested(move || { if let Some(window)=weak.upgrade() { if window.get_busy() { window.set_dialog_confirmation(false); window.set_dialog_can_launch(false); window.set_dialog_title("Operation in progress".into()); window.set_dialog_body("The active transaction has started replacing managed files. Keep this window open until the verified result appears.".into()); window.set_dialog_visible(true); return slint::CloseRequestResponse::KeepWindowShown; } } slint::CloseRequestResponse::HideWindow });
    }
    window.run()?;
    Ok(())
}

fn quote_cli_value(value: &Path) -> String {
    format!("\"{}\"", value.display())
}

fn elevated_install_arguments(
    data_dir: &Path,
    game_dir: Option<&Path>,
    allow_unknown: bool,
    distribution: Option<Distribution>,
    install_obs: bool,
    firewall_public: bool,
) -> String {
    let mut args = vec![
        "--data-dir".into(),
        quote_cli_value(data_dir),
        "--install-now".into(),
        "--method".into(),
        match distribution {
            Some(Distribution::Standalone) => "standalone",
            Some(Distribution::BeatblockPlus) => "beatblock-plus",
            None => "automatic",
        }
        .into(),
    ];
    if let Some(path) = game_dir {
        args.extend(["--game-dir".into(), quote_cli_value(path)]);
    }
    if allow_unknown {
        args.push("--allow-unknown-build".into());
    }
    if install_obs {
        args.push("--install-obs".into());
    }
    if firewall_public {
        args.push("--firewall-public".into());
    }
    args.join(" ")
}

fn elevate_simple(
    data_dir: &Path,
    flag: &str,
    remove_data: bool,
    weak: slint::Weak<InstallerWindow>,
) -> Result<()> {
    let operation_file = data_dir.join(format!("operations/{}.json", uuid::Uuid::new_v4()));
    if let Some(parent) = operation_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut arguments = format!("--data-dir {} {flag}", quote_cli_value(data_dir));
    if remove_data {
        arguments.push_str(" --remove-user-data");
    }
    Installer::request_elevated_with_progress(&arguments, &operation_file, |event| {
        post_progress(weak.clone(), event)
    })
}

fn needs_elevation(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::PermissionDenied
                || matches!(io.raw_os_error(), Some(5 | 740))
            {
                return true;
            }
        }
        let m = cause.to_string().to_ascii_lowercase();
        m.contains("access is denied")
            || m.contains("permission denied")
            || m.contains("requires administrator")
    })
}

#[cfg(windows)]
fn copy_to_clipboard(text: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    };
    const CF_UNICODETEXT: u32 = 13;
    let wide = std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            anyhow::bail!("Windows clipboard is busy");
        }
        EmptyClipboard();
        let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if memory.is_null() {
            CloseClipboard();
            anyhow::bail!("clipboard allocation failed");
        }
        let target = GlobalLock(memory) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(memory);
        if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
            CloseClipboard();
            anyhow::bail!("clipboard transfer failed");
        }
        CloseClipboard();
    }
    Ok(())
}
#[cfg(not(windows))]
fn copy_to_clipboard(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard is supported on Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn access_denied_requests_elevation_but_validation_errors_do_not() {
        let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(needs_elevation(&denied));
        assert!(!needs_elevation(&anyhow::anyhow!(
            "unsupported fingerprint"
        )));
    }
    #[test]
    fn elevated_arguments_preserve_selected_target() {
        let args = elevated_install_arguments(
            Path::new(r"C:\Users\Player\App Data\BBT"),
            Some(Path::new(r"C:\Program Files\Beatblock")),
            true,
            Some(Distribution::BeatblockPlus),
            true,
            true,
        );
        assert!(args.contains(r#"--game-dir "C:\Program Files\Beatblock""#));
        assert!(args.contains("--method beatblock-plus"));
        assert!(args.contains("--install-obs"));
    }
}
