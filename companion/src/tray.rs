#[cfg(windows)]
pub fn run(url: String, exports: std::path::PathBuf) {
    std::thread::spawn(move || {
        use tray_icon::{
            menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
            TrayIconBuilder,
        };
        let menu = Menu::new();
        let open_console = MenuItem::new("Open broadcast console", true, None);
        let open_exports = MenuItem::new("Open exports folder", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let open_id = open_console.id().clone();
        let exports_id = open_exports.id().clone();
        let quit_id = quit.id().clone();
        let _ = menu.append(&open_console);
        let _ = menu.append(&open_exports);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);
        let _tray = TrayIconBuilder::new()
            .with_tooltip("Beatblock Together")
            .with_menu(Box::new(menu))
            .build()
            .ok();
        loop {
            if let Ok(event) =
                MenuEvent::receiver().recv_timeout(std::time::Duration::from_millis(250))
            {
                if event.id == open_id {
                    let _ = open::that(&url);
                } else if event.id == exports_id {
                    let _ = open::that(&exports);
                } else if event.id == quit_id {
                    std::process::exit(0);
                }
            }
        }
    });
}
#[cfg(not(windows))]
pub fn run(_url: String, _exports: std::path::PathBuf) {}
