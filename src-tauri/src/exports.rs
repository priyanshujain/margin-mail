// Taking your mail and your decisions out, and putting the decisions back.
//
// This is the module that makes "ownership is a feature, not a slogan" true. Mail leaves as mbox,
// which every mail client on earth can read, and the decisions leave as the journal itself, which
// is the same thing the backup store carries and the same thing another device absorbs. There is no
// export format invented here: the point of exporting is that somebody else can read it, and a
// shape nobody else knows is not an export.
//
// Nothing here asks the provider for anything. An export is of what is on the device, which is what
// the storage window decided, and saying so is more honest than quietly downloading a mailbox.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use rusqlite::Connection;
use tauri::Manager;

use crate::db::Db;
use crate::state;

/// Where an export lands. The downloads directory, because that is where a person looks for a file
/// they asked an app to make, and because writing into the app's own data directory would put the
/// export somewhere uninstalling deletes.
fn downloads(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| e.to_string())
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------------------------
// Mail, as mbox
// ---------------------------------------------------------------------------------------------

/// The mboxo escape, which is the one every reader agrees on: a body line that begins with `From `
/// gains a `>`, because that sequence at the start of a line is what separates one message from the
/// next and a message quoting an email header would otherwise split in two.
fn escape_from_lines(raw: &[u8], out: &mut Vec<u8>) {
    let mut at_line_start = true;
    let mut i = 0;
    while i < raw.len() {
        if at_line_start && raw[i..].starts_with(b"From ") {
            out.push(b'>');
        }
        out.push(raw[i]);
        at_line_start = raw[i] == b'\n';
        i += 1;
    }
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
}

/// One message as an mbox entry: the separator line, then the message, then a blank line.
///
/// The separator carries the envelope sender and a date in asctime form, which is what the format
/// asks for and what readers parse. A message with no raw bytes is skipped rather than written as
/// an empty entry, because the mirror keeps headers for messages whose bodies it never fetched and
/// an entry with no message in it is worse than an absence.
fn mbox_entry(from: &str, date_ms: i64, raw: &[u8], out: &mut Vec<u8>) {
    let when = chrono::DateTime::from_timestamp_millis(date_ms)
        .unwrap_or_default()
        .format("%a %b %e %H:%M:%S %Y");
    let sender = if from.is_empty() { "MAILER-DAEMON" } else { from };
    out.extend_from_slice(format!("From {sender} {when}\n").as_bytes());
    escape_from_lines(raw, out);
    out.push(b'\n');
}

fn write_mbox(conn: &Connection, path: &PathBuf) -> Result<u32, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.from_address, m.date_ms, b.raw FROM messages m
             JOIN bodies b ON b.message_id = m.id
             WHERE b.raw IS NOT NULL
             ORDER BY m.date_ms ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut written = 0u32;
    for row in rows {
        let (from, date_ms, raw) = row.map_err(|e| e.to_string())?;
        let mut entry = Vec::with_capacity(raw.len() + 96);
        mbox_entry(&from, date_ms, &raw, &mut entry);
        file.write_all(&entry).map_err(|e| e.to_string())?;
        written += 1;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(written)
}

/// Every message on the device for one account, as mbox, in the downloads directory.
///
/// What leaves is what is here: a body the mirror never fetched is not in the file, because the
/// export is of the device rather than of the mailbox. The Data section says so beside the button.
#[tauri::command(async)]
pub fn export_mbox(app: tauri::AppHandle, account_id: String) -> Result<String, String> {
    let db = app.try_state::<Db>().ok_or("the mirror is not open yet")?;
    let safe: String = account_id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let path = downloads(&app)?.join(format!("margin-mail-{safe}-{}.mbox", stamp()));
    db.with(&account_id, |conn| write_mbox(conn, &path))?;
    Ok(path.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------------------------
// The decisions, as the journal
// ---------------------------------------------------------------------------------------------

/// Every decision on this device, as the journal, one account per key.
///
/// The journal rather than the tables, because the tables are a view of it and the journal is what
/// another device can absorb without losing the order things happened in. It is the same shape the
/// backup store carries, which is deliberate: one export format, one import path, one thing to get
/// right.
#[tauri::command(async)]
pub fn export_state(app: tauri::AppHandle) -> Result<String, String> {
    let db = app.try_state::<Db>().ok_or("the mirror is not open yet")?;
    let mut accounts = serde_json::Map::new();

    for account_id in db.on_disk() {
        let records = db.with(&account_id, |conn| {
            let mut all = Vec::new();
            for device in state::merge::devices(conn)? {
                all.extend(state::merge::export(conn, &device, 0)?);
            }
            Ok(all)
        })?;
        accounts.insert(
            account_id,
            serde_json::to_value(&records).map_err(|e| e.to_string())?,
        );
    }

    let document = serde_json::json!({
        "kind": "margin-mail-state",
        "version": 1,
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "accounts": accounts,
    });
    let path = downloads(&app)?.join(format!("margin-mail-decisions-{}.json", stamp()));
    crate::library::atomic_write(
        &path,
        serde_json::to_string_pretty(&document)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    Ok(path.to_string_lossy().to_string())
}

/// Reads an export back in.
///
/// Absorbing rather than replacing, so importing into an account that has been used since is a
/// merge and not a loss: the journal's own last-writer-wins settles every key, exactly as it does
/// when two devices meet. Importing the same file twice changes nothing, which is the same property
/// that lets a device catch up after a month offline.
#[tauri::command(async)]
pub fn import_state(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let db = app.try_state::<Db>().ok_or("the mirror is not open yet")?;
    let raw = fs::read_to_string(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    let document: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    if document.get("kind").and_then(|k| k.as_str()) != Some("margin-mail-state") {
        return Err("that file is not a Margin Mail export".to_string());
    }
    let accounts = document
        .get("accounts")
        .and_then(|a| a.as_object())
        .ok_or("that export has no accounts in it")?;

    let here = db.on_disk();
    for (account_id, records) in accounts {
        // An account that is not connected here has nowhere for its decisions to go, and creating a
        // database for it would be creating an account nobody added.
        if !here.contains(account_id) {
            continue;
        }
        let records: Vec<state::journal::Record> =
            serde_json::from_value(records.clone()).map_err(|e| e.to_string())?;
        db.with(account_id, |conn| {
            state::merge::absorb(conn, &records).map(|_| ())
        })?;
    }
    crate::emit_store_changed(&app, "threads state screener");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The keymap
// ---------------------------------------------------------------------------------------------

/// The keymap is a file rather than a table of pickers, which is a decision `docs/keyboard.md`
/// makes: a person who wants to remap a key wants to see the whole map at once.
pub fn keymap_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(crate::library::app_data_dir(app)?.join("keymap.json"))
}

const KEYMAP_TEMPLATE: &str = r#"{
  "$comment": [
    "Margin Mail's keymap. Every command and its default key is in docs/keyboard.md, and the",
    "shortcuts sheet behind ? is generated from whatever is in force, so a remapped key is what",
    "the buttons print.",
    "",
    "One entry per command you want to change: the command's id, and the keys it should answer to.",
    "A combo is modifiers in cmd+ctrl+alt+shift order and then the key, so cmd+shift+a. Delete this",
    "file, or press Reset in Settings, to go back to the defaults."
  ],
  "bindings": {}
}
"#;

/// The path, creating the file with its explanation the first time somebody asks for it. A settings
/// screen that offers to open a file has to be sure there is one to open.
#[tauri::command(async)]
pub fn keymap_path(app: tauri::AppHandle) -> Result<String, String> {
    let path = keymap_file(&app)?;
    if !path.exists() {
        crate::library::atomic_write(&path, KEYMAP_TEMPLATE.as_bytes())?;
    }
    Ok(path.to_string_lossy().to_string())
}

/// Back to the defaults, which is the template rather than an absence: a file that is there and
/// empty is easier to understand than one that has gone.
#[tauri::command(async)]
pub fn keymap_reset(app: tauri::AppHandle) -> Result<(), String> {
    let path = keymap_file(&app)?;
    crate::library::atomic_write(&path, KEYMAP_TEMPLATE.as_bytes())
}

#[cfg(test)]
mod tests;
