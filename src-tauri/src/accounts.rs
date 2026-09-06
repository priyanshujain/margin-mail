// Which Google accounts this install has a token for, and the handful of facts about each one
// that are not mail.
//
// A file, `accounts.json` in the app data directory, rather than a table. Every account owns a
// database of its own under `accounts/<id>/`, so a table of accounts would have to live in one of
// them and be read before the others could be opened, and the first thing the app does on launch
// is ask which accounts there are. A file answers that before any database is open.
//
// What is not here matters as much as what is. The refresh token is sealed in `google::secrets`
// and never written to this file. The storage window and the signature belong to settings and to
// the state database, which are the places that already own per account decisions.
//
// This is the list of accounts that have a token. `Db::on_disk` is the list that have a database,
// and a removal takes the entry here before it touches anything else, so the two lists differ for
// as long as a removal takes and neither is authoritative on its own.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::dto::{Account, AccountKind, MailConfig};
use crate::library::{app_data_dir, atomic_write};

const FILE: &str = "accounts.json";

/// The stylesheet defines `--hue-1` to `--hue-8`, so those are the only values worth storing.
const HUES: usize = 8;

/// One row of the registry. `granted_scopes` is what Google said it granted, not what was asked
/// for, because granular consent means those are different lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub email: String,
    /// Absent in a registry written before there was a second kind, which is what the default is
    /// for: every account that predates IMAP is a Google one, and the file is upgraded in place
    /// the next time anything writes it.
    #[serde(default)]
    pub kind: AccountKind,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub granted_scopes: Vec<String>,
    /// An IMAP account's servers. `None` on a Google account, where the mailbox is reached over
    /// the API and there is nothing to configure. The passwords are not here: they are sealed
    /// alongside the refresh tokens in `google::secrets`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<MailConfig>,
    pub added_ms: i64,
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join(FILE))
}

pub fn list(app: &tauri::AppHandle) -> Result<Vec<Account>, String> {
    Ok(read(&path(app)?)?
        .into_iter()
        .map(|entry| account(app, entry))
        .collect())
}

pub fn find(app: &tauri::AppHandle, account_id: &str) -> Result<Option<Entry>, String> {
    Ok(read(&path(app)?)?
        .into_iter()
        .find(|entry| entry.id == account_id))
}

/// What Google granted this account, for the features that have to check before they act rather
/// than discover it as a 403 halfway through.
pub fn granted_scopes(app: &tauri::AppHandle, account_id: &str) -> Result<Vec<String>, String> {
    find(app, account_id)?
        .map(|entry| entry.granted_scopes)
        .ok_or_else(|| format!("Account {account_id} is not connected."))
}

/// A completed consent, arriving from `google::auth::finish`.
///
/// An account that is already here keeps its name and its colour: consent is re-run to pick up a
/// scope, and having the avatar change colour because somebody agreed to a calendar permission
/// would be a strange thing to watch. The scopes are replaced wholesale, since the new grant is
/// the only one that exists now.
pub fn upsert(
    app: &tauri::AppHandle,
    id: &str,
    email: &str,
    name: &str,
    granted_scopes: Vec<String>,
) -> Result<(), String> {
    let path = path(app)?;
    let mut entries = read(&path)?;
    match entries.iter_mut().find(|entry| entry.id == id) {
        Some(entry) => {
            entry.email = email.to_string();
            entry.granted_scopes = granted_scopes;
        }
        None => {
            restore_kept(app, id)?;
            let color = next_hue(&entries);
            entries.push(Entry {
                id: id.to_string(),
                email: email.to_string(),
                kind: AccountKind::Google,
                name: name.to_string(),
                color,
                granted_scopes,
                servers: None,
                added_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
    }
    write(&path, &entries)
}

/// An IMAP account arriving from `imap::connect`, which is the other way into this registry.
///
/// Deliberately a second function rather than a wider `upsert`: a Google account has scopes and no
/// servers, an IMAP account has servers and no scopes, and one function taking both would be four
/// arguments that are each meaningless in half the calls.
pub fn upsert_imap(
    app: &tauri::AppHandle,
    id: &str,
    email: &str,
    name: &str,
    servers: MailConfig,
) -> Result<(), String> {
    let path = path(app)?;
    let mut entries = read(&path)?;
    match entries.iter_mut().find(|entry| entry.id == id) {
        Some(entry) => {
            entry.email = email.to_string();
            entry.servers = Some(servers);
        }
        None => {
            restore_kept(app, id)?;
            let color = next_hue(&entries);
            entries.push(Entry {
                id: id.to_string(),
                email: email.to_string(),
                kind: AccountKind::Imap,
                name: name.to_string(),
                color,
                granted_scopes: Vec::new(),
                servers: Some(servers),
                added_ms: chrono::Utc::now().timestamp_millis(),
            });
        }
    }
    write(&path, &entries)
}

/// An account that was removed with its data kept gets that data back on its way in, before the
/// first sync pass can write a fresh, empty pair over the place it belongs. `Db` is absent in a
/// test and in any build that has not opened one yet, when there is nothing set aside either.
fn restore_kept(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    match app.try_state::<crate::db::Db>() {
        Some(db) => db.restore(id),
        None => Ok(()),
    }
}

pub fn remove(app: &tauri::AppHandle, account_id: &str) -> Result<(), String> {
    let path = path(app)?;
    let mut entries = read(&path)?;
    let before = entries.len();
    entries.retain(|entry| entry.id != account_id);
    if entries.len() == before {
        return Ok(());
    }
    write(&path, &entries)
}

#[tauri::command(async)]
pub fn accounts_list(app: tauri::AppHandle) -> Result<Vec<Account>, String> {
    list(&app)
}

#[tauri::command(async)]
pub fn account_set_color(
    app: tauri::AppHandle,
    account_id: String,
    color: String,
) -> Result<(), String> {
    let path = path(&app)?;
    let mut entries = read(&path)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == account_id)
        .ok_or_else(|| format!("Account {account_id} is not connected."))?;
    entry.color = hue(&color)?;
    write(&path, &entries)?;
    crate::emit_store_changed(&app, "accounts");
    Ok(())
}

#[tauri::command(async)]
pub fn account_set_name(
    app: tauri::AppHandle,
    account_id: String,
    name: String,
) -> Result<(), String> {
    let path = path(&app)?;
    let mut entries = read(&path)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == account_id)
        .ok_or_else(|| format!("Account {account_id} is not connected."))?;
    let name = name.trim();
    if name.is_empty() {
        return Err("An account needs a name.".to_string());
    }
    entry.name = name.to_string();
    write(&path, &entries)?;
    crate::emit_store_changed(&app, "accounts");
    Ok(())
}

/// Every account in this file has a token, so `connected` is true without going and looking. A
/// probe would be a file read and a key derivation on every `store-changed`, which sync emits on
/// every pass, for an answer nothing acts on: a secret that has really gone surfaces at the next
/// `valid_access_token` as a sync error, which is the honest place to learn it.
///
/// `window_days` is not this file's to know. The window is a device setting and `settings.json`
/// owns it; this registry owns the id, the address, the name, the colour and the grant. The DTO
/// carries both, so the value is fetched from the one that owns it on the way past.
fn account(app: &tauri::AppHandle, entry: Entry) -> Account {
    Account {
        window_days: crate::settings::window_days(app, &entry.id),
        id: entry.id,
        email: entry.email,
        kind: entry.kind,
        name: entry.name,
        color: entry.color,
        connected: true,
        granted_scopes: entry.granted_scopes,
    }
}

fn hue(color: &str) -> Result<String, String> {
    let n = color.strip_prefix("hue-").and_then(|n| n.parse::<usize>().ok());
    match n {
        Some(n) if (1..=HUES).contains(&n) => Ok(color.to_string()),
        // The stylesheet owns the value: a hex stored here would reach the avatar as an unknown
        // token name and render as nothing at all.
        _ => Err(format!("{color} is not one of hue-1 to hue-{HUES}.")),
    }
}

/// The lowest hue nobody is using, rather than the next one along, so removing an account and
/// adding another reuses the colour that went spare instead of leaving two accounts to collide
/// eight connections later. The avatar hue is how the unified inbox says whose mail a row is, so
/// two accounts sharing one while a colour is going spare is the thing to avoid.
fn next_hue(entries: &[Entry]) -> String {
    for n in 1..=HUES {
        let hue = format!("hue-{n}");
        if !entries.iter().any(|entry| entry.color == hue) {
            return hue;
        }
    }
    format!("hue-{}", entries.len() % HUES + 1)
}

/// A missing file is an install with no accounts, which is every install once. A malformed one is
/// not: it is the only record of which accounts exist, and overwriting it would silently orphan
/// every sealed token and every database on disk.
fn read(path: &Path) -> Result<Vec<Entry>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(&text).map_err(|e| format!("could not read {}: {e}", path.display()))
}

fn write(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, color: &str) -> Entry {
        Entry {
            id: id.to_string(),
            email: format!("{id}@example.test"),
            kind: AccountKind::Google,
            name: id.to_string(),
            color: color.to_string(),
            granted_scopes: vec!["https://www.googleapis.com/auth/gmail.modify".to_string()],
            servers: None,
            added_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_registry_survives_a_write_and_a_read() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(FILE);

        let written = vec![entry("11829", "hue-1"), entry("44021", "hue-2")];
        write(&path, &written).expect("write");

        let read_back = read(&path).expect("read");
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[1].id, "44021");
        assert_eq!(read_back[1].email, "44021@example.test");
        assert_eq!(read_back[1].color, "hue-2");
        assert_eq!(
            read_back[0].granted_scopes,
            vec!["https://www.googleapis.com/auth/gmail.modify".to_string()]
        );
        assert_eq!(read_back[0].added_ms, 1_700_000_000_000);
    }

    #[test]
    fn no_file_yet_is_an_install_with_no_accounts() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert!(read(&dir.path().join(FILE)).expect("read").is_empty());
    }

    #[test]
    fn a_malformed_registry_is_an_error_rather_than_an_empty_list() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(FILE);
        std::fs::write(&path, "{ not json").expect("write");
        assert!(read(&path).is_err());
    }

    #[test]
    fn hues_are_handed_out_in_order() {
        let mut entries: Vec<Entry> = Vec::new();
        for n in 1..=HUES {
            let hue = next_hue(&entries);
            assert_eq!(hue, format!("hue-{n}"));
            entries.push(entry(&format!("account-{n}"), &hue));
        }
        // A ninth account has to share, and starts the cycle again rather than inventing hue-9.
        assert_eq!(next_hue(&entries), "hue-1");
    }

    #[test]
    fn a_removed_accounts_hue_is_reused_before_a_new_one() {
        let mut entries = vec![
            entry("a", "hue-1"),
            entry("b", "hue-2"),
            entry("c", "hue-3"),
        ];
        entries.retain(|entry| entry.id != "b");
        assert_eq!(next_hue(&entries), "hue-2");
    }

    #[test]
    fn only_the_eight_token_names_are_a_colour() {
        assert_eq!(hue("hue-1").expect("hue-1"), "hue-1");
        assert_eq!(hue("hue-8").expect("hue-8"), "hue-8");
        assert!(hue("hue-9").is_err());
        assert!(hue("hue-0").is_err());
        assert!(hue("#8c6f4a").is_err());
        assert!(hue("").is_err());
    }
}
