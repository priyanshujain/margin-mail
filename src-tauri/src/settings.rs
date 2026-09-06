// The device half of what docs/settings.md splits in two: `settings.json` in the app data
// directory, replaced whole through `library::atomic_write` so a crash mid-write leaves the old
// file rather than half a new one.
//
// The roaming half is mostly not here. Sender rules belong to the state database and reach a second
// device through its journal, and this file never sees them. The two exceptions are the signature
// and the instant intro text: they are edited here because this is where the settings screen is,
// and `roam` copies each change into the journal so that they follow the person the way
// docs/settings.md says they should. This file stays the one that is read.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tauri::Manager;

use crate::dto::{Account, AccountSettings, BackupSettings, FontRef, Settings, SnoozeTimes};
use crate::library::{app_data_dir, atomic_write};
use crate::mirror::write::DEFAULT_WINDOW_DAYS;

const FILE: &str = "settings.json";

/// What an install that has never opened this screen looks like.
///
/// Notifications are off, the Screener is on and the backup store is none, which is the product's
/// position rather than a placeholder: nothing interrupts you and nothing leaves the machine until
/// somebody says so.
///
/// The dock badge is the one thing here that is on. It interrupts nobody: it is a count of what is
/// waiting in the Inbox, sitting where somebody has to go and look at it, and a person who wants
/// their unread mail counted at them by an app they have to find the setting for has not been
/// given the thing they asked for.
pub fn defaults() -> Settings {
    Settings {
        theme: "system".to_string(),
        font_ui: FontRef::Bundled {
            id: "hanken".to_string(),
        },
        font_text: FontRef::Bundled {
            id: "literata".to_string(),
        },
        text_size: 15,
        reading_pane: true,
        density: "comfortable".to_string(),

        accounts: Vec::new(),
        attachment_cache_mb: 512,
        prefetch_bodies: true,

        remote_images: "ask".to_string(),
        link_cleaning: true,

        screener_enabled: true,
        hold_replies: false,
        suggestions: true,

        snooze_times: SnoozeTimes {
            later_today_hours: 3,
            tomorrow_at: 8 * 60,
            weekend_at: 9 * 60,
            next_week_at: 8 * 60,
        },
        swipe_right: "reply-later".to_string(),
        swipe_left: "set-aside".to_string(),
        feed_auto_trash_days: 0,

        undo_delay_secs: 10,
        reply_all_default: false,
        instant_intro: "Thank you for the introduction, moving you to bcc.".to_string(),

        badge: true,
        notifications: true,
        notify_places: Vec::new(),

        backup: BackupSettings {
            store: "none".to_string(),
            configured: false,
            last_backup_ms: None,
            has_phrase: false,
            r2_bucket: None,
            r2_endpoint: None,
        },
    }
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join(FILE))
}

/// Everything this file holds, with a row for every account that is connected.
pub fn load(app: &tauri::AppHandle) -> Result<Settings, String> {
    let path = path(app)?;
    let mut settings = load_at(&path)?;
    if reconcile(&mut settings, &crate::accounts::list(app).unwrap_or_default()) {
        write(&path, &settings)?;
    }
    Ok(settings)
}

/// How far back this account keeps mail. Zero is everything.
///
/// Infallible on purpose, and the one place in this module that is. `accounts_list` calls it for
/// every row it returns, and an account list that refused to render because a settings file was
/// unreadable would hide the screen the person needs to fix it from. `settings_get` is where a
/// malformed file is reported.
pub fn window_days(app: &tauri::AppHandle, account_id: &str) -> u32 {
    path(app)
        .and_then(|path| read(&path))
        .ok()
        .flatten()
        .and_then(|settings| {
            settings
                .accounts
                .into_iter()
                .find(|row| row.account_id == account_id)
                .map(|row| row.window_days)
        })
        .unwrap_or(DEFAULT_WINDOW_DAYS as u32)
}

/// The window for one account, chosen on the panel that waits for its first sync and written the
/// way the Settings row writes it, so the same file owns it from then on. The engine is told the
/// same way a change from Settings tells it; on a mirror with nothing in it yet that queues a
/// backfill nobody needs, and the first sync drops that on its way past because it lists the
/// whole window itself.
pub fn set_window_days(app: &tauri::AppHandle, account_id: &str, days: u32) -> Result<(), String> {
    let path = path(app)?;
    let before = load(app)?;
    let mut after = before.clone();
    let row = after
        .accounts
        .iter_mut()
        .find(|row| row.account_id == account_id)
        .ok_or_else(|| format!("Account {account_id} is not connected."))?;
    row.window_days = days;
    write(&path, &after)?;
    for (account_id, days) in window_changes(&before, &after) {
        crate::sync::window_set(app, &account_id, days)?;
    }
    crate::emit_store_changed(app, "settings accounts");
    Ok(())
}

#[tauri::command(async)]
pub fn settings_get(app: tauri::AppHandle) -> Result<Settings, String> {
    load(&app)
}

/// A partial patch, merged into what is on disk rather than replacing it, so a screen that wants to
/// change the theme sends the theme and not a whole settings object it might be a version behind on.
#[tauri::command(async)]
pub fn settings_set(app: tauri::AppHandle, patch: Value) -> Result<Settings, String> {
    let path = path(&app)?;
    let before = load(&app)?;
    let after = merge(&before, patch)?;
    write(&path, &after)?;

    for (account_id, days) in window_changes(&before, &after) {
        crate::sync::window_set(&app, &account_id, days)?;
    }

    // The roaming half. A signature and the instant intro are things a person would be annoyed to
    // retype on a new machine, which is the test `docs/settings.md` sets for what roams, so they go
    // through the state journal as well as into this file. The file is still what is read: this is
    // a copy that travels, not a second home.
    roam(&app, &before, &after)?;

    // `accounts` as well as `settings`: the window is a field of the account DTO, so the list the
    // header and the settings screen both read has moved too.
    crate::emit_store_changed(&app, "settings accounts");
    Ok(after)
}

/// The families installed on this machine, for the Appearance section's two pickers, alongside the
/// six margin-shared bundles.
///
/// Loading the system fonts parses every face on disk, which is tens of milliseconds on a laptop
/// and rather more on a workstation with a type library, so it goes off the main thread.
#[tauri::command(async)]
pub fn system_fonts() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut families: Vec<String> = db
        .faces()
        .filter_map(|face| face.families.first().map(|(family, _)| family.clone()))
        .collect();
    families.sort();
    families.dedup();
    families
}

// ---------------------------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------------------------

/// Reads the file, writing the defaults the first time there is nothing to read.
fn load_at(path: &Path) -> Result<Settings, String> {
    match read(path)? {
        Some(settings) => Ok(settings),
        None => {
            let settings = defaults();
            write(path, &settings)?;
            Ok(settings)
        }
    }
}

/// `None` is an install that has never written the file. A file that will not parse is an error
/// instead: it holds decisions somebody made, and replacing it with the defaults would throw them
/// away to make a screen render.
fn read(path: &Path) -> Result<Option<Settings>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("could not read {}: {e}", path.display()))
}

fn write(path: &Path, settings: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    atomic_write(path, text.as_bytes())
}

fn merge(base: &Settings, patch: Value) -> Result<Settings, String> {
    let mut value = serde_json::to_value(base).map_err(|e| e.to_string())?;
    merge_into(&mut value, patch);
    serde_json::from_value(value).map_err(|e| format!("that is not a settings patch: {e}"))
}

/// Objects merge key by key and everything else replaces, so a patch naming one snooze time leaves
/// the other three alone while a patch naming the account list means that list.
fn merge_into(base: &mut Value, patch: Value) {
    match patch {
        Value::Object(fields) => match base {
            Value::Object(target) => {
                for (key, value) in fields {
                    match target.get_mut(&key) {
                        Some(slot) => merge_into(slot, value),
                        None => {
                            target.insert(key, value);
                        }
                    }
                }
            }
            other => *other = Value::Object(fields),
        },
        other => *base = other,
    }
}

/// The accounts whose window moved, which is the only thing in this file another module has to be
/// told about. A row that is new here carries the default, which is what the mirror already had.
fn window_changes(before: &Settings, after: &Settings) -> Vec<(String, i64)> {
    after
        .accounts
        .iter()
        .filter(|now| {
            before
                .accounts
                .iter()
                .any(|was| was.account_id == now.account_id && was.window_days != now.window_days)
        })
        .map(|now| (now.account_id.clone(), now.window_days as i64))
        .collect()
}

/// A row appears here when an account is connected, and takes its name and colour from the registry
/// every time it is read. Two files holding a display name is two answers to one question, and
/// `accounts.json` is the one that owns it.
fn reconcile(settings: &mut Settings, accounts: &[Account]) -> bool {
    let mut changed = false;
    for account in accounts {
        match settings
            .accounts
            .iter_mut()
            .find(|row| row.account_id == account.id)
        {
            Some(row) => {
                if row.name != account.name || row.color != account.color {
                    row.name = account.name.clone();
                    row.color = account.color.clone();
                    changed = true;
                }
            }
            None => {
                settings.accounts.push(AccountSettings {
                    account_id: account.id.clone(),
                    name: account.name.clone(),
                    color: account.color.clone(),
                    window_days: DEFAULT_WINDOW_DAYS as u32,
                    signature: String::new(),
                    aliases: Vec::new(),
                });
                changed = true;
            }
        }
    }
    changed
}

/// Writes the settings that should follow the person into the state journal, where the backup
/// store and a second device can reach them.
///
/// Only on a change, because every write here is an event in a log that is kept forever, and a
/// settings screen that saved on every keystroke would fill it with the same value.
fn roam(app: &tauri::AppHandle, before: &Settings, after: &Settings) -> Result<(), String> {
    let Some(db) = app.try_state::<crate::db::Db>() else {
        return Ok(());
    };

    for account in &after.accounts {
        let was = before
            .accounts
            .iter()
            .find(|held| held.account_id == account.account_id);

        let signature_moved = was.map(|held| held.signature != account.signature).unwrap_or(!account.signature.is_empty());
        let intro_moved = before.instant_intro != after.instant_intro;
        if !signature_moved && !intro_moved {
            continue;
        }

        // An account with no database yet is an account that has not synced, and a preference for
        // one is not worth creating a database over. It lands the next time this runs.
        if !db.on_disk().contains(&account.account_id) {
            continue;
        }
        db.with(&account.account_id, |conn| {
            if signature_moved {
                crate::state::write::set_pref(conn, "signature", &account.signature)?;
            }
            if intro_moved {
                crate::state::write::set_pref(conn, "instant-intro", &after.instant_intro)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(id: &str, name: &str, color: &str) -> Account {
        Account {
            id: id.to_string(),
            email: format!("{id}@example.test"),
            kind: crate::dto::AccountKind::Google,
            name: name.to_string(),
            color: color.to_string(),
            connected: true,
            granted_scopes: Vec::new(),
            window_days: 0,
        }
    }

    fn row(id: &str, days: u32) -> AccountSettings {
        AccountSettings {
            account_id: id.to_string(),
            name: id.to_string(),
            color: "hue-1".to_string(),
            window_days: days,
            signature: String::new(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn the_defaults_are_written_on_the_first_read() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(FILE);

        let settings = load_at(&path).expect("first read");
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.undo_delay_secs, 10);
        assert!(settings.badge, "the badge is on out of the box");
        assert!(path.exists());

        // And the file that was written is the one that comes back next time.
        let again = load_at(&path).expect("second read");
        assert_eq!(again.instant_intro, settings.instant_intro);
    }

    #[test]
    fn a_file_from_before_the_switch_over_everything_loads_with_it_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(FILE);
        let mut value = serde_json::to_value(defaults()).expect("value");
        value
            .as_object_mut()
            .expect("object")
            .remove("notifications")
            .expect("the key was there to remove");
        std::fs::write(&path, serde_json::to_string(&value).expect("json")).expect("write");

        let settings = load_at(&path).expect("read");
        assert!(settings.notifications, "an older file means what it meant: nothing was turned off");
    }

    #[test]
    fn a_patch_merges_rather_than_replaces() {
        let base = defaults();
        let after = merge(
            &base,
            json!({ "theme": "dark", "snoozeTimes": { "tomorrowAt": 420 } }),
        )
        .expect("merge");

        assert_eq!(after.theme, "dark");
        assert_eq!(after.snooze_times.tomorrow_at, 420);
        // Everything the patch did not name is what it was, at both levels.
        assert_eq!(after.snooze_times.weekend_at, base.snooze_times.weekend_at);
        assert_eq!(after.undo_delay_secs, base.undo_delay_secs);
        assert!(matches!(after.font_text, FontRef::Bundled { ref id } if id == "literata"));
    }

    #[test]
    fn a_list_in_a_patch_is_the_list_rather_than_an_addition() {
        let mut base = defaults();
        base.accounts = vec![row("acct-1", 30), row("acct-2", 90)];
        let after = merge(&base, json!({ "accounts": [] })).expect("merge");
        assert!(after.accounts.is_empty());
    }

    #[test]
    fn a_malformed_file_is_reported_rather_than_replaced() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(FILE);
        std::fs::write(&path, "{ not json").expect("write");

        assert!(read(&path).is_err());
        assert!(load_at(&path).is_err());
        // The decisions somebody made are still on disk to be repaired.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn a_window_change_is_handed_to_the_sync_engine() {
        let mut before = defaults();
        before.accounts = vec![row("acct-1", 30), row("acct-2", 90)];

        let after = merge(
            &before,
            json!({ "accounts": [
                { "accountId": "acct-1", "name": "acct-1", "color": "hue-1", "windowDays": 365, "signature": "", "aliases": [] },
                { "accountId": "acct-2", "name": "acct-2", "color": "hue-1", "windowDays": 90, "signature": "Sent from Margin", "aliases": [] }
            ] }),
        )
        .expect("merge");

        // Only the one that moved, and the second account's signature is not a window change.
        assert_eq!(
            window_changes(&before, &after),
            vec![("acct-1".to_string(), 365)]
        );
        assert!(window_changes(&before, &before).is_empty());
    }

    #[test]
    fn a_connected_account_gets_a_row_and_keeps_the_registrys_name() {
        let mut settings = defaults();
        let accounts = vec![account("acct-1", "Priyanshu Jain", "hue-4")];

        assert!(reconcile(&mut settings, &accounts));
        assert_eq!(settings.accounts.len(), 1);
        assert_eq!(settings.accounts[0].name, "Priyanshu Jain");
        assert_eq!(settings.accounts[0].window_days, DEFAULT_WINDOW_DAYS as u32);
        assert!(!reconcile(&mut settings, &accounts));

        // The registry is renamed, so the row follows it rather than the other way round.
        let renamed = vec![account("acct-1", "Work", "hue-4")];
        assert!(reconcile(&mut settings, &renamed));
        assert_eq!(settings.accounts[0].name, "Work");
    }
}
