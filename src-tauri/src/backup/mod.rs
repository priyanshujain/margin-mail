// The backup, which is the state journal encrypted, put somewhere the person chose, and brought
// back.
//
// store.rs   put a blob at a name, get a blob by name, list names under a prefix. Three operations
// drive.rs   that trait over the Drive client in `google::drive`
// r2.rs      that trait over S3 signature version four, for somebody who runs their own
// crypto.rs  what leaves the device and what the store can learn from it
// phrase.rs  the twenty-four words and the key they derive
//
// Nothing here decides anything. `state::merge` already knows how two logs meet, `state::journal`
// already guarantees that replaying a log rebuilds the tables, and this package is the transport
// and the encryption around that. What it adds is a rhythm: a device uploads its own segments under
// its own device id and downloads every other device's, so two machines that never met still land
// on the same tables, and a machine that was off for a month replays what it missed in one pass.
//
// The one hazard worth naming, because it is a product rule rather than a bug to fix here. A second
// device attaches with the phrase; it must never turn backup on by itself. Two devices that each
// generate their own phrase for the same account write two backups under two keys into one folder,
// and neither can read the other's. That is what the sentence in the Backup section of settings is
// for, and a segment that will not decrypt says so by name rather than being skipped, because a
// backup that quietly ignores what it cannot read is a backup that quietly loses half of itself.

pub mod crypto;
pub mod drive;
pub mod phrase;
pub mod r2;
pub mod store;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::db::Db;
use crate::dto::BackupSettings;
use crate::state::{device, journal::Record, merge};

use crypto::Key;
use store::BackupStore;

/// Events per segment. Small enough that an upload cut off halfway has lost very little work and
/// large enough that a month of decisions is a handful of files rather than a directory listing
/// nobody can read.
const SEGMENT_RECORDS: usize = 500;

const SUFFIX: &str = ".seg";

/// A hash rather than the address, because a folder listing in somebody's Drive should not be a
/// list of the email addresses they have accounts for. It is the same on every device, which is
/// what lets a second one find the first one's folder from the phrase and the account alone.
///
/// Truncated to 128 bits: this names one folder among a person's handful and is not a secret, and
/// a name that fits on one line is worth more here than the other half of the digest.
pub fn account_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"margin-mail backup account v1");
    hasher.update(email.trim().to_lowercase().as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    })
}

/// `<account-hash>/<device-id>/<first>-<last>.seg`.
///
/// The name carries a range of sequence numbers and nothing else. It cannot say what is in the
/// segment, because everything a record holds is inside the ciphertext, and it does say what a
/// device needs in order to skip a download: a segment ending at or below what this device already
/// holds for that device is a segment it has already absorbed. That is what makes catching up a
/// month cost one listing and only the files that were missed.
fn segment_name(account_hash: &str, device_id: &str, first: i64, last: i64) -> String {
    format!("{account_hash}/{device_id}/{first:012}-{last:012}{SUFFIX}")
}

/// What a name says. Anything that does not parse belongs to somebody else and is left alone.
fn parse_name(name: &str) -> Option<(String, i64, i64)> {
    let mut parts = name.split('/');
    let _account = parts.next()?;
    let device = parts.next()?;
    let file = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (first, last) = file.strip_suffix(SUFFIX)?.split_once('-')?;
    Some((device.to_string(), first.parse().ok()?, last.parse().ok()?))
}

/// The journal side of a pass, reached through a closure.
///
/// A `&Connection` may not be held across an await and the transport in the middle of a pass is
/// asynchronous, so the database work happens in these closures and the network work happens
/// between them. Deliberately not `sync::Store`, which is the same shape: that one means the
/// mirror and this one means the journal, and sharing the name would couple two packages through a
/// word that means different things in each.
pub trait Journal: Sync {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String>;
}

/// The app's journal: the shared connection pool, narrowed to one account.
pub struct Account<'a> {
    pub db: &'a Db,
    pub account_id: &'a str,
}

impl Journal for Account<'_> {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        self.db.with(self.account_id, f)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    pub uploaded: usize,
    pub downloaded: usize,
    pub absorbed: usize,
    pub skipped: usize,
}

/// One account's whole exchange with the store: what is new here goes up, what is new anywhere else
/// comes down and is absorbed.
///
/// Both halves are driven by the listing rather than by a local watermark, so there is one source
/// of truth about what the store holds and it is the store. A pass that died halfway through its
/// uploads leaves whole segments behind it, and the next pass sees them and carries on from there.
///
/// Reading happens before writing, which matters on the pass a restore runs: a key that cannot open
/// what is already in the folder fails before it has added anything of its own, so a phrase typed
/// wrongly cannot leave a second backup's worth of segments that nothing can read.
pub async fn pass<J: Journal, S: BackupStore>(
    journal: &J,
    store: &S,
    key: &Key,
    account_hash: &str,
) -> Result<Pass, String> {
    let held = store.list(&format!("{account_hash}/")).await?;
    let device_id = journal.with(device::device_id)?;
    let mut pass = Pass::default();

    let high: HashMap<String, i64> = journal.with(merge::high_water)?.into_iter().collect();
    let mut wanted: Vec<&String> = held
        .iter()
        .filter(|name| match parse_name(name) {
            Some((device, _, last)) => {
                device != device_id && last > *high.get(&device).unwrap_or(&0)
            }
            None => false,
        })
        .collect();
    wanted.sort();

    let mut records: Vec<Record> = Vec::new();
    for name in wanted {
        let sealed = store.get(name).await?;
        let plain = crypto::open(key, name, &sealed)?;
        let text = String::from_utf8(plain).map_err(|_| format!("{name} is not a segment"))?;
        records.extend(merge::decode(&text)?);
        pass.downloaded += 1;
    }

    if !records.is_empty() {
        let report = journal.with(|conn| merge::absorb(conn, &records))?;
        pass.absorbed = report.applied;
        pass.skipped = report.skipped;
    }

    let uploaded_to = held
        .iter()
        .filter_map(|name| parse_name(name))
        .filter(|(device, _, _)| device == &device_id)
        .map(|(_, _, last)| last)
        .max()
        .unwrap_or(0);

    // Only this device's own events, which is why two logs meeting is a union rather than a
    // conflict: no other device can have written under this sequence.
    let mine = journal.with(|conn| merge::export(conn, &device_id, uploaded_to))?;
    for chunk in mine.chunks(SEGMENT_RECORDS) {
        let (Some(first), Some(last)) = (chunk.first(), chunk.last()) else {
            continue;
        };
        let name = segment_name(account_hash, &device_id, first.seq, last.seq);
        let body = crypto::seal(key, &name, merge::encode(chunk)?.as_bytes())?;
        store.put(&name, &body).await?;
        pass.uploaded += 1;
    }
    Ok(pass)
}

// ---------------------------------------------------------------------------------------------
// The commands
// ---------------------------------------------------------------------------------------------

/// Which store the settings file names, with whatever it needs to reach it.
enum Chosen {
    None,
    Drive,
    R2(r2::Config),
}

fn chosen(app: &tauri::AppHandle) -> Result<Chosen, String> {
    match crate::settings::load(app)?.backup.store.as_str() {
        "drive" => Ok(Chosen::Drive),
        "r2" => match r2::stored()? {
            Some(config) => Ok(Chosen::R2(config)),
            None => Err(
                "The R2 credentials are not on this device any more. Enter them again.".to_string(),
            ),
        },
        _ => Ok(Chosen::None),
    }
}

/// Writes the non-secret half of the backup settings back into `settings.json`, through the
/// settings module's own patch path so there is one writer of that file.
fn record(app: &tauri::AppHandle, patch: serde_json::Value) -> Result<BackupSettings, String> {
    let settings =
        crate::settings::settings_set(app.clone(), serde_json::json!({ "backup": patch }))?;
    Ok(with_key_state(settings.backup))
}

/// `has_phrase` is not a setting, it is whether a key is sealed on this device, so it is answered
/// from the key store every time rather than trusted from the file.
fn with_key_state(mut settings: BackupSettings) -> BackupSettings {
    settings.has_phrase = crypto::have_key();
    settings
}

/// One account's pass against whichever store is turned on. The store is built here rather than
/// once for the run because the Drive one holds an account: it signs its requests with that
/// account's token, and `drive.file` means it can only see the files it created itself.
async fn pass_for_account(
    app: &tauri::AppHandle,
    chosen: &Chosen,
    key: &Key,
    account_id: &str,
    email: &str,
) -> Result<Pass, String> {
    let db = app
        .try_state::<Db>()
        .ok_or_else(|| "the databases are not open yet".to_string())?;
    let journal = Account {
        db: db.inner(),
        account_id,
    };
    let hash = account_hash(email);
    match chosen {
        Chosen::None => Err("No backup store is turned on.".to_string()),
        Chosen::Drive => {
            let store = drive::Drive::new(app.clone(), account_id);
            pass(&journal, &store, key, &hash).await
        }
        Chosen::R2(config) => {
            let store = r2::R2::new(config.clone());
            pass(&journal, &store, key, &hash).await
        }
    }
}

#[tauri::command]
pub async fn backup_status(app: tauri::AppHandle) -> Result<BackupSettings, String> {
    Ok(with_key_state(crate::settings::load(&app)?.backup))
}

/// `store` is "drive", "r2" or "none". R2 takes the four S3 fields and nothing else; Drive takes
/// none, because it uses the account that is already connected.
#[tauri::command]
pub async fn backup_configure(
    app: tauri::AppHandle,
    store: String,
    config: HashMap<String, String>,
) -> Result<BackupSettings, String> {
    match store.as_str() {
        "drive" => {
            let accounts = crate::accounts::list(&app)?;
            if accounts.is_empty() {
                return Err("Connect an account before turning backup on.".to_string());
            }
            // Granular consent means the Drive tick box is the user's to clear, so this is asked
            // now rather than discovered as a failure during the first backup.
            if !accounts
                .iter()
                .any(|account| crate::google::drive::has_scope(&account.granted_scopes))
            {
                return Err(crate::google::drive::REAUTH_MESSAGE.to_string());
            }
            record(
                &app,
                serde_json::json!({
                    "store": "drive",
                    "configured": true,
                    "r2Bucket": serde_json::Value::Null,
                    "r2Endpoint": serde_json::Value::Null
                }),
            )
        }
        "r2" => {
            let config = r2::Config::from_fields(&config)?;
            r2::remember(&config)?;
            record(
                &app,
                serde_json::json!({
                    "store": "r2",
                    "configured": true,
                    "r2Bucket": config.bucket,
                    "r2Endpoint": config.endpoint
                }),
            )
        }
        "none" => {
            // The key stays. It is what reads the segments already up there, and turning the store
            // off is not a decision to make those unreadable. The credentials for somebody else's
            // bucket do not stay, because they are not ours to keep once they are not in use.
            r2::forget()?;
            record(
                &app,
                serde_json::json!({
                    "store": "none",
                    "configured": false,
                    "r2Bucket": serde_json::Value::Null,
                    "r2Endpoint": serde_json::Value::Null
                }),
            )
        }
        other => Err(format!("{other} is not a backup store")),
    }
}

#[tauri::command]
pub async fn backup_now(app: tauri::AppHandle) -> Result<BackupSettings, String> {
    let key = crypto::stored()?.ok_or(
        "Backup has no recovery phrase yet. Turn backup on and write the phrase down first.",
    )?;
    let chosen = chosen(&app)?;
    if matches!(chosen, Chosen::None) {
        return Err("No backup store is turned on.".to_string());
    }
    let mut absorbed = 0;
    for account in crate::accounts::list(&app)? {
        let pass = pass_for_account(&app, &chosen, &key, &account.id, &account.email).await?;
        absorbed += pass.absorbed;
    }

    let settings = record(
        &app,
        serde_json::json!({ "lastBackupMs": chrono::Utc::now().timestamp_millis() }),
    )?;
    if absorbed > 0 {
        crate::emit_store_changed(&app, "state threads");
    }
    Ok(settings)
}

/// Shown once, at setup, and never returned again.
///
/// Literally never: the phrase is generated here, its key is sealed, and the phrase itself is
/// dropped at the end of this function. Argon2 does not run backwards, so there is nothing left on
/// the device that could print it a second time even if a screen asked. That is the property being
/// bought, and it is why the second call is an explanation rather than a repeat.
#[tauri::command]
pub async fn backup_phrase(app: tauri::AppHandle) -> Result<String, String> {
    if crypto::have_key() {
        return Err("The recovery phrase is shown once and is not kept anywhere, so it cannot be shown again. Backup on this device carries on working without it; the phrase is only needed to attach another device or to restore onto a new one.".to_string());
    }
    let phrase = phrase::generate()?;
    crypto::remember(&phrase::derive(&phrase)?)?;
    record(&app, serde_json::json!({ "hasPhrase": true }))?;
    Ok(phrase)
}

/// Attaches this device to an existing backup, which is also how a lost device is replaced.
///
/// The phrase becomes the key, one pass brings down every other device's segments and replays
/// them, and only then is the key sealed: a phrase that turns out not to be this backup's leaves
/// the device exactly as it was rather than half attached to something it cannot read. There is
/// nothing else to a restore. The journal rebuilds the tables and the mirror is derived from the
/// provider, so what comes back is every decision and none of the mail, which fills itself in on
/// the next sync.
#[tauri::command]
pub async fn backup_restore(app: tauri::AppHandle, phrase: String) -> Result<(), String> {
    let key = phrase::derive(&phrase)?;
    let chosen = chosen(&app)?;
    if matches!(chosen, Chosen::None) {
        return Err("Choose where the backup is kept before restoring from it.".to_string());
    }

    for account in crate::accounts::list(&app)? {
        pass_for_account(&app, &chosen, &key, &account.id, &account.email).await?;
    }
    crypto::remember(&key)?;

    record(
        &app,
        serde_json::json!({
            "hasPhrase": true,
            "lastBackupMs": chrono::Utc::now().timestamp_millis()
        }),
    )?;
    crate::emit_store_changed(&app, "state threads");
    Ok(())
}
