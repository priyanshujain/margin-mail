mod dto;
mod library;

#[cfg(desktop)]
use tauri::menu::{Menu, MenuItemBuilder, MenuItemKind, PredefinedMenuItem, SubmenuBuilder};
#[cfg(desktop)]
use tauri::Runtime;
use tauri::{Emitter, Manager};

/// The frontend treats this as an invalidation signal with a scope: the reason names what moved
/// (`threads`, `thread:<key>`, `accounts`, `settings`, `state`), so a note landing does not make
/// the list refetch its bodies. Mailspring's typed delta stream in one string.
pub fn emit_store_changed(app: &tauri::AppHandle, reason: &str) {
    let _ = app.emit("store-changed", reason);
}

#[cfg(desktop)]
fn build_menu<R: Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(handle)?;

    let new_message = MenuItemBuilder::with_id("new-message", "New Message")
        .accelerator("CmdOrCtrl+N")
        .build(handle)?;
    let command_palette = MenuItemBuilder::with_id("command-palette", "Command Palette…")
        .accelerator("CmdOrCtrl+K")
        .build(handle)?;
    let sync_now = MenuItemBuilder::with_id("sync-now", "Sync Now")
        .accelerator("CmdOrCtrl+R")
        .build(handle)?;
    let check_updates =
        MenuItemBuilder::with_id("check-updates", "Check for Updates…").build(handle)?;
    let settings = MenuItemBuilder::with_id("settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(handle)?;
    let search = MenuItemBuilder::with_id("search", "Search…")
        .accelerator("CmdOrCtrl+F")
        .build(handle)?;
    let shortcuts = MenuItemBuilder::with_id("shortcuts", "Keyboard Shortcuts")
        .accelerator("CmdOrCtrl+/")
        .build(handle)?;
    let report_issue = MenuItemBuilder::with_id("report-issue", "Report an Issue…").build(handle)?;

    let submenus: Vec<_> = menu
        .items()?
        .into_iter()
        .filter_map(|item| match item {
            MenuItemKind::Submenu(submenu) => Some(submenu),
            _ => None,
        })
        .collect();

    let find_submenu = |name: &str| {
        submenus
            .iter()
            .find(|submenu| submenu.text().map(|t| t == name).unwrap_or(false))
            .cloned()
    };

    match find_submenu("File") {
        Some(submenu) => {
            submenu.prepend_items(&[
                &new_message,
                &command_palette,
                &PredefinedMenuItem::separator(handle)?,
                &sync_now,
                &PredefinedMenuItem::separator(handle)?,
            ])?;
        }
        None => {
            let submenu = SubmenuBuilder::new(handle, "File")
                .item(&new_message)
                .item(&command_palette)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&sync_now)
                .build()?;
            menu.insert(&submenu, 1)?;
        }
    }

    if let Some(edit) = find_submenu("Edit") {
        edit.append_items(&[&PredefinedMenuItem::separator(handle)?, &search])?;
    }

    if let Some(help) = find_submenu("Help") {
        help.append_items(&[&shortcuts, &report_issue])?;
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(app_submenu) = submenus.first() {
            app_submenu.insert(&check_updates, 1)?;
            app_submenu.insert(&settings, 3)?;
            app_submenu.insert(&PredefinedMenuItem::separator(handle)?, 4)?;
        }
        if let Some(view) = find_submenu("View") {
            let inbox = MenuItemBuilder::with_id("place-inbox", "Inbox")
                .accelerator("CmdOrCtrl+1")
                .build(handle)?;
            let feed = MenuItemBuilder::with_id("place-feed", "Feed")
                .accelerator("CmdOrCtrl+2")
                .build(handle)?;
            let paper_trail = MenuItemBuilder::with_id("place-paper-trail", "Paper Trail")
                .accelerator("CmdOrCtrl+3")
                .build(handle)?;
            let toggle_pane = MenuItemBuilder::with_id("toggle-pane", "Reading Pane")
                .accelerator("CmdOrCtrl+\\")
                .build(handle)?;
            view.prepend_items(&[
                &inbox,
                &feed,
                &paper_trail,
                &PredefinedMenuItem::separator(handle)?,
                &toggle_pane,
                &PredefinedMenuItem::separator(handle)?,
            ])?;
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(file) = find_submenu("File") {
            file.append_items(&[&PredefinedMenuItem::separator(handle)?, &check_updates])?;
        }
        if let Some(edit) = find_submenu("Edit") {
            edit.append_items(&[&settings])?;
        }
    }

    Ok(menu)
}

/// The other half of closing on macOS. The red button and Cmd+W hide the window rather than destroy
/// it, the way every mail client does: the app stays in the Dock with sync still running, and Cmd+Q
/// is what quits. AppKit does nothing of its own for a Dock click when it can see no window.
#[cfg(target_os = "macos")]
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The package manager that owns this install, when one does. The Nix wrapper sets it to "nix":
/// the binary lives in a read-only store there, so the updater may announce a version but not
/// install it.
#[tauri::command]
fn packaged_by() -> Option<String> {
    std::env::var("MARGIN_MAIL_PACKAGED_BY")
        .ok()
        .filter(|manager| !manager.is_empty())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // generate_context! first, so the updater plugin registers only when the merged config actually
    // has an `updater` key. Ported from margin's lib.rs.
    let context = tauri::generate_context!();

    #[cfg_attr(mobile, allow(unused_mut))]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init());

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_process::init());
        if context.config().plugins.0.contains_key("updater") {
            builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
        }
    }

    builder = builder.setup(|_app| Ok(()));

    #[cfg(desktop)]
    {
        builder = builder
            .menu(|handle| build_menu(handle))
            .on_menu_event(|app, event| {
                if matches!(
                    event.id().0.as_str(),
                    "new-message"
                        | "command-palette"
                        | "sync-now"
                        | "check-updates"
                        | "settings"
                        | "search"
                        | "shortcuts"
                        | "place-inbox"
                        | "place-feed"
                        | "place-paper-trail"
                        | "toggle-pane"
                        | "report-issue"
                ) {
                    app.emit("menu-action", event.id().0.as_str()).ok();
                }
            });
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder.on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    }

    let app = builder
        .invoke_handler(tauri::generate_handler![packaged_by])
        .build(context)
        .expect("error while building Margin Mail");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            show_main_window(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
