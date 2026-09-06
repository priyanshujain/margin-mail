// `pub` throughout rather than private modules: this crate is a library with a thin binary in
// front of it, the modules are the parts, and a contract type that nothing has consumed yet is a
// contract type rather than dead code.
pub mod accounts;
pub mod attachments;
pub mod imap;
pub mod backup;
pub mod badge;
pub mod clips;
pub mod contacts;
pub mod db;
pub mod decisions;
pub mod drafts;
pub mod dto;
pub mod exports;
#[cfg(test)]
pub mod fixtures;
pub mod google;
pub mod invites;
pub mod library;
pub mod log;
pub mod mime;
pub mod mirror;
pub mod notify;
pub mod piles;
pub mod provider;
pub mod routing;
pub mod sanitize;
pub mod screener;
pub mod send;
pub mod settings;
pub mod snooze;
pub mod state;
pub mod sync;
pub mod undo;
pub mod unsubscribe;

#[cfg(desktop)]
use tauri::menu::{Menu, MenuItemBuilder, MenuItemKind, PredefinedMenuItem, SubmenuBuilder};
#[cfg(desktop)]
use tauri::Runtime;
use tauri::{Emitter, Manager};

/// The frontend treats this as an invalidation signal with a scope: the reason names what moved
/// (`threads`, `thread:<key>`, `accounts`, `settings`, `state`), so a note landing does not make
/// the list refetch its bodies. Mailspring's typed delta stream in one string.
///
/// The dock badge hangs off the same signal for the same reason: everything that can change what
/// is waiting in the Inbox already announces itself here, and a second notification path would be
/// a second thing to remember to call.
pub fn emit_store_changed(app: &tauri::AppHandle, reason: &str) {
    let _ = app.emit("store-changed", reason);
    badge::on_store_changed(app, reason);
}

#[cfg(desktop)]
fn build_menu<R: Runtime>(handle: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(handle)?;

    let new_message = MenuItemBuilder::with_id("compose", "New Message")
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
    let guide = MenuItemBuilder::with_id("guide", "Margin Mail Guide").build(handle)?;
    let tour = MenuItemBuilder::with_id("tour", "Take the Tour").build(handle)?;
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

    // Kept, because the later platform sections have to reach it and `find_submenu` reads a list
    // taken before this ran: on a platform whose default menu has no File, looking it up again
    // there finds nothing and Check for Updates lands nowhere.
    #[cfg_attr(target_os = "macos", allow(unused_variables))]
    let file = match find_submenu("File") {
        Some(submenu) => {
            submenu.prepend_items(&[
                &new_message,
                &command_palette,
                &PredefinedMenuItem::separator(handle)?,
                &sync_now,
                &PredefinedMenuItem::separator(handle)?,
            ])?;
            submenu
        }
        None => {
            let submenu = SubmenuBuilder::new(handle, "File")
                .item(&new_message)
                .item(&command_palette)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&sync_now)
                .build()?;
            // First, because File is the first menu everywhere. Index 1 only on macOS, where the
            // application menu holds index 0 and nothing may go before it.
            menu.insert(&submenu, if cfg!(target_os = "macos") { 1 } else { 0 })?;
            submenu
        }
    };

    if let Some(edit) = find_submenu("Edit") {
        edit.append_items(&[&PredefinedMenuItem::separator(handle)?, &search])?;
    }

    // The Help menu in the order macOS expects: what the app can tell you first, then the way to
    // tell somebody about the app.
    if let Some(help) = find_submenu("Help") {
        help.append_items(&[
            &guide,
            &tour,
            &shortcuts,
            &PredefinedMenuItem::separator(handle)?,
            &report_issue,
        ])?;
    }

    // The three places and the reading pane, which belong under View on every platform. Only macOS
    // is given a View menu to put them in; everywhere else one is built below.
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

    #[cfg(target_os = "macos")]
    {
        if let Some(app_submenu) = submenus.first() {
            app_submenu.insert(&check_updates, 1)?;
            app_submenu.insert(&settings, 3)?;
            app_submenu.insert(&PredefinedMenuItem::separator(handle)?, 4)?;
        }
        if let Some(view) = find_submenu("View") {
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
        file.append_items(&[&PredefinedMenuItem::separator(handle)?, &check_updates])?;
        if let Some(edit) = find_submenu("Edit") {
            edit.append_items(&[&PredefinedMenuItem::separator(handle)?, &settings])?;
        }

        let view = SubmenuBuilder::new(handle, "View")
            .item(&inbox)
            .item(&feed)
            .item(&paper_trail)
            .item(&PredefinedMenuItem::separator(handle)?)
            .item(&toggle_pane)
            .build()?;
        // After Edit, which is where View sits in every menu bar that draws one. Falling back to
        // the third slot rather than to the end, because the end is past Help.
        let after_edit = menu
            .items()?
            .iter()
            .position(|item| match item {
                MenuItemKind::Submenu(submenu) => {
                    submenu.text().map(|t| t == "Edit").unwrap_or(false)
                }
                _ => false,
            })
            .map_or(2, |index| index + 1);
        menu.insert(&view, after_edit)?;
    }

    Ok(menu)
}

/// The other half of closing on macOS. The red button and Cmd+W hide the window rather than destroy
/// it, the way every mail client does: the app stays in the Dock with sync still running, and Cmd+Q
/// is what quits. AppKit does nothing of its own for a Dock click when it can see no window.
#[cfg(target_os = "macos")]
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
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
/// Hands the engine the mailbox behind an account. The engine only ever speaks through the trait,
/// so this is the single place in the app that decides the mailbox behind an account is Gmail
/// rather than an IMAP server.
fn attach_account(handle: &tauri::AppHandle, account: &dto::Account) {
    match account.kind {
        dto::AccountKind::Google => {
            sync::attach(&account.id, google::gmail::Gmail::new(handle.clone(), &account.id));
        }
        // An IMAP account without its servers on disk is a half-written registry rather than an
        // account, so it is left unattached and says so in the account chip.
        dto::AccountKind::Imap => {
            if let Ok(Some(entry)) = accounts::find(handle, &account.id) {
                if let Some(servers) = entry.servers {
                    sync::attach(
                        &account.id,
                        imap::provider::Imap::new(handle.clone(), &account.id, servers),
                    );
                }
            }
        }
    }
}

/// The step between an account being written and its mail arriving: the window it is to hold,
/// then the provider behind it, then the first pass, now rather than at the next tick.
///
/// The window comes first because the first sync reads it, and it is asked for rather than
/// assumed because a month is a guess and a year of a busy mailbox is a long wait nobody was
/// told about. Until this is called the account is in the registry and nowhere else; a launch in
/// between attaches it with whatever window the settings hold, which is the default.
#[tauri::command]
async fn account_start(
    app: tauri::AppHandle,
    account_id: String,
    window_days: u32,
) -> Result<(), String> {
    settings::set_window_days(&app, &account_id, window_days)?;
    let account = accounts::list(&app)?
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| format!("Account {account_id} is not connected."))?;
    attach_account(&app, &account);
    sync::kick(&app, &account_id);
    Ok(())
}

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

    builder = builder.manage(google::AuthState::default()).setup(|app| {
        let handle = app.handle();

        // The mirror and the state database, one connection per account, opened lazily by the
        // first read. Managed here because every command that touches SQLite reaches for it.
        app.manage(db::Db::open(handle)?);

        // Start fresh is reversible for seven days, which is longer than this process lives, so its
        // records are on disk and this is what puts a surviving one back within reach of `z`.
        let _ = mirror::fresh::rearm(handle.state::<db::Db>().inner());

        // Restores a session per sealed token, so a launch does not send anybody back to consent.
        google::auth::init_sessions(handle);

        // The certificates somebody has already decided about, which is what stops a local bridge
        // asking the same question every restart.
        if let Ok(dir) = library::app_data_dir(handle) {
            imap::init(dir);
        }

        // One provider per connected account, then the poll loop. An account added while the app
        // is up joins through `account_start`, once the window it is to hold has been chosen.
        for account in accounts::list(handle).unwrap_or_default() {
            attach_account(handle, &account);
        }
        sync::setup(handle.clone());

        // The dock keeps whatever number it was left with across a quit, so a launch has to say
        // what is true now rather than waiting for the first thing to change.
        badge::refresh(handle);

        // The notification centre wants its delegate before anything is posted, and a click on a
        // notification comes back through it.
        #[cfg(target_os = "macos")]
        notify::macos::install(handle.clone());

        #[cfg(mobile)]
        listen_for_redirects(handle);
        #[cfg(target_os = "ios")]
        if let Some(window) = handle.get_webview_window("main") {
            stop_uikit_shrinking_the_viewport(&window);
        }
        #[cfg(target_os = "android")]
        if let Some(window) = handle.get_webview_window("main") {
            watch_for_the_consent_tab_closing(&window);
        }
        Ok(())
    });

    #[cfg(desktop)]
    {
        builder = builder
            .menu(|handle| build_menu(handle))
            .on_menu_event(|app, event| {
                if matches!(
                    event.id().0.as_str(),
                    "compose"
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
                        | "guide"
                        | "tour"
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
        .invoke_handler(tauri::generate_handler![
            // Accounts and consent
            accounts::accounts_list,
            account_start,
            log::log_note,
            accounts::account_set_color,
            accounts::account_set_name,
            google::account_connect,
            google::account_grant,
            google::account_remove,
            // Reading
            mirror::threads_list,
            imap::imap_discover,
            imap::imap_test,
            imap::imap_connect,
            imap::imap_servers,
            imap::imap_trust_cert,
            imap::imap_forget_cert,
            attachments::message_show_images,
            attachments::attachment_data_url,
            attachments::attachment_save,
            attachments::attachment_open,
            mirror::thread_view,
            mirror::thread_opened,
            mirror::thread_hydrate,
            mirror::labels_list,
            mirror::storage_used,
            // Triage
            mirror::flags_set,
            mirror::mark_all_seen,
            mirror::label_apply,
            mirror::label_move,
            mirror::mirror_clear,
            mirror::fresh::start_fresh,
            undo::undo_last,
            undo::undo_token,
            // Routing and the Screener
            screener::screener_list,
            screener::screener_decide,
            screener::screener_clear_all,
            screener::screener_seed,
            // The piles and their friends
            piles::pile_toggle,
            snooze::snooze_set,
            snooze::snooze_clear,
            snooze::snooze_evaluate,
            decisions::note_add,
            decisions::note_update,
            decisions::note_delete,
            decisions::thread_rename,
            decisions::thread_merge,
            decisions::thread_unmerge,
            decisions::thread_ignore,
            decisions::thread_notify,
            clips::clip_save,
            clips::clips_list,
            clips::clip_delete,
            clips::files_list,
            unsubscribe::unsubscribe,
            // Contacts
            contacts::contact_card,
            contacts::contact_update,
            contacts::contacts_list,
            contacts::contacts_suggest,
            // Writing
            drafts::draft_save,
            drafts::draft_get,
            drafts::draft_delete,
            send::send,
            send::send_now,
            send::outbox_list,
            invites::invite_respond,
            // Backup
            backup::backup_status,
            backup::backup_configure,
            backup::backup_now,
            backup::backup_phrase,
            backup::backup_restore,
            // Sync
            sync::sync_now,
            sync::sync_status,
            sync::sync_flush,
            sync::sync_backfill,
            sync::search,
            sync::search_provider,
            // Settings
            settings::settings_get,
            settings::settings_set,
            settings::system_fonts,
            notify::notify_test,
            notify::notify_permission,
            notify::notify_request,
            notify::notify_open_settings,
            notify::notify_take,

            // Taking it all out again
            exports::export_mbox,
            exports::export_state,
            exports::import_state,
            exports::keymap_path,
            exports::keymap_reset,
            packaged_by
        ])
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
