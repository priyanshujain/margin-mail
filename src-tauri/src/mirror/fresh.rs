// Start fresh, and the one reversal in this app that outlives the process.
//
// Marking everything older than a moment as seen is the only bulk write to the provider Margin
// Mail ever proposes. docs/features.md section 2 promises it is reversible for seven days, and
// that promise is the whole reason this module exists rather than one more call to `flags_set`.
//
// The ordinary undo stack in `mirror` is a `VecDeque` in memory bounded at fifty. That is exactly
// right for taking an archive back ten seconds after it: nobody expects a keystroke to be
// reversible after a quit, and fifty is deeper than anybody reaches in a session. It is exactly
// wrong for seven days. It does not survive a restart, and fifty ordinary actions push the entry
// off the bottom whether or not the week is up. A promise measured in seconds is kept by a stack;
// a promise measured in days has to be kept by the database.
//
// So the record goes in the mirror's own `meta`, keyed by the moment it was made, and the token
// handed back is that key. What is kept is the ids that were unseen rather than the action, so the
// reversal is idempotent: the in-memory entry and the record on disk describe the same change, and
// either one may be the one that runs.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::dto::{FlagPatch, Undo};
use crate::sync::{self, outbox};

use super::write;

/// How long the offer stands, from docs/features.md section 2.
pub const KEEP_MS: i64 = 7 * write::DAY_MS;

const KEY_PREFIX: &str = "start-fresh:";

/// One offer to take a start fresh back. The ids are a JSON array in a single `meta` row because
/// the schema is frozen and a table of its own is not on offer; a mailbox big enough for that to
/// matter is also one where a megabyte for a week is not the expensive part.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    made_at: i64,
    older_than_ms: i64,
    label: String,
    /// The messages that were unseen before. All this reversal has to put back, because start
    /// fresh only ever removes UNREAD.
    unseen: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// The change
// ---------------------------------------------------------------------------------------------

/// Marks every thread whose latest message is older than a moment as seen, and records what was
/// unseen so it can be put back.
///
/// The bin and the spam pile are left alone. This is the one bulk write the app proposes, so it
/// proposes as little as it can get away with, and nothing in either place was ever going to show
/// up in New for you.
pub fn start(
    conn: &Connection,
    account_id: &str,
    older_than_ms: i64,
    now: i64,
) -> Result<Undo, String> {
    prune(conn, now)?;

    let mut ids: Vec<String> = Vec::new();
    let mut threads: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.provider_thread_id
                 FROM messages m
                 JOIN threads t ON t.provider_thread_id = m.provider_thread_id
                 WHERE t.latest_ms < ?1 AND t.trashed = 0 AND t.spam = 0
                   AND m.seen = 0 AND m.draft = 0
                 ORDER BY m.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([older_than_ms], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, thread_id) = row.map_err(|e| e.to_string())?;
            if !threads.contains(&thread_id) {
                threads.push(thread_id);
            }
            ids.push(id);
        }
    }

    let label = label_for(threads.len());
    if ids.is_empty() {
        // Nothing marked is nothing to take back. A seven day promise about a change that was
        // never made is not worth a row, so there is no record and no token.
        return Ok(Undo {
            token: String::new(),
            label,
            undo_ms: 0,
        });
    }

    let patch = FlagPatch {
        seen: Some(true),
        ..FlagPatch::default()
    };
    let prior = write::prior_flags(conn, &ids)?;
    write::apply_flags(conn, &ids, &patch)?;
    outbox::queue_flags(conn, &ids, &patch, None)?;

    let token = free_key(conn, now)?;
    let record = Record {
        made_at: now,
        older_than_ms,
        label: label.clone(),
        unseen: ids,
    };
    write::meta_set(
        conn,
        &token,
        &serde_json::to_string(&record).map_err(|e| e.to_string())?,
    )?;

    // The stack as well as the disk, so the toast's Undo and `z` reach this through the ordinary
    // path in the session that made it. The record is what is left when they no longer can.
    super::undo_push(super::UndoEntry {
        token: token.clone(),
        label: label.clone(),
        prior: vec![(account_id.to_string(), prior)],
    });

    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

/// Puts back exactly what was unseen, and forgets the offer. A message that has been marked seen
/// again since, by hand or by the provider, is put back too: this is "as it was", which is the same
/// rule the ordinary undo follows.
pub fn reverse(conn: &Connection, token: &str) -> Result<u32, String> {
    let record = read_record(conn, token)?.ok_or("that change can no longer be taken back")?;
    let restoring = still_seen(conn, &record.unseen)?;

    if !restoring.is_empty() {
        let patch = FlagPatch {
            seen: Some(false),
            ..FlagPatch::default()
        };
        write::apply_flags(conn, &restoring, &patch)?;
        outbox::queue_flags(conn, &restoring, &patch, None)?;
    }

    write::meta_clear(conn, token)?;
    let _ = super::undo_take(token);
    Ok(restoring.len() as u32)
}

/// Drops every offer that has run out of its seven days. Called whenever this module is touched,
/// which is the only sweep a record ever gets.
pub fn prune(conn: &Connection, now: i64) -> Result<u32, String> {
    let mut gone = 0;
    for (key, record) in records(conn)? {
        if now - record.made_at >= KEEP_MS {
            write::meta_clear(conn, &key)?;
            let _ = super::undo_take(&key);
            gone += 1;
        }
    }
    Ok(gone)
}

/// What is still reversible in one account, newest first. Prunes what has expired and forgets what
/// has already been taken back, whichever path took it.
pub fn pending(conn: &Connection, now: i64) -> Result<Vec<Undo>, String> {
    prune(conn, now)?;
    let mut out = Vec::new();
    for (key, record) in records(conn)? {
        if still_seen(conn, &record.unseen)?.is_empty() {
            write::meta_clear(conn, &key)?;
            continue;
        }
        out.push(Undo {
            token: key,
            label: record.label,
            undo_ms: 0,
        });
    }
    out.reverse();
    Ok(out)
}

/// Puts every surviving offer back within reach of `undo_token`, which reads the in-memory stack
/// and knows nothing about this file. One call at startup is what makes the seven days real across
/// a restart; without it a record is on disk and unreachable.
pub fn rearm(db: &Db) -> Result<u32, String> {
    let now = write::now_ms();
    let mut offered = 0;
    for (account_id, _) in sync::accounts(db, None) {
        offered += db.with(&account_id, |conn| {
            let mut count = 0u32;
            for offer in pending(conn, now)? {
                let record = match read_record(conn, &offer.token)? {
                    Some(record) => record,
                    None => continue,
                };
                offer_again(super::UndoEntry {
                    token: offer.token,
                    label: offer.label,
                    prior: vec![(account_id.clone(), as_unseen(conn, &record.unseen)?)],
                });
                count += 1;
            }
            Ok(count)
        })?;
    }
    Ok(offered)
}

/// Reverses by token, whichever account holds the record.
pub fn undo(app: &tauri::AppHandle, token: &str) -> Result<u32, String> {
    let db = super::db_of(app)?;
    for (account_id, _) in sync::accounts(db.inner(), None) {
        let done = db.with(&account_id, |conn| match read_record(conn, token)? {
            Some(_) => reverse(conn, token).map(Some),
            None => Ok(None),
        })?;
        if let Some(done) = done {
            crate::emit_store_changed(app, "threads");
            return Ok(done);
        }
    }
    Err("that change can no longer be taken back".to_string())
}

// ---------------------------------------------------------------------------------------------
// The records
// ---------------------------------------------------------------------------------------------

fn records(conn: &Connection) -> Result<Vec<(String, Record)>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM meta WHERE key LIKE ?1 ORDER BY key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([format!("{KEY_PREFIX}%")], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        if let Ok(record) = serde_json::from_str::<Record>(&value) {
            out.push((key, record));
        }
    }
    Ok(out)
}

fn read_record(conn: &Connection, token: &str) -> Result<Option<Record>, String> {
    if !token.starts_with(KEY_PREFIX) {
        return Ok(None);
    }
    Ok(write::meta_get(conn, token)?.and_then(|raw| serde_json::from_str::<Record>(&raw).ok()))
}

/// Which of the recorded messages are still on the device and still seen, which is exactly the set
/// a reversal has to touch. Everything else has been evicted or is already unseen.
fn still_seen(conn: &Connection, ids: &[String]) -> Result<Vec<String>, String> {
    let json = serde_json::to_string(ids).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id FROM messages
             WHERE seen = 1 AND id IN (SELECT value FROM json_each(?1)) ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([json], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The label set each message should be restored to: whatever it carries now, plus the unread it
/// had before. Anything else that happened to it in the meantime is not this reversal's business.
fn as_unseen(conn: &Connection, ids: &[String]) -> Result<Vec<write::PriorFlags>, String> {
    let mut prior = write::prior_flags(conn, ids)?;
    for held in &mut prior {
        if !held.labels.iter().any(|label| label == write::LABEL_UNREAD) {
            held.labels.push(write::LABEL_UNREAD.to_string());
        }
    }
    Ok(prior)
}

/// A key nothing else is using, in this account's `meta` or in the stack the whole app shares.
fn free_key(conn: &Connection, now: i64) -> Result<String, String> {
    let mut at = now;
    loop {
        let key = format!("{KEY_PREFIX}{at}");
        let taken = write::meta_get(conn, &key)?.is_some() || in_stack(&key);
        if !taken {
            return Ok(key);
        }
        at += 1;
    }
}

fn in_stack(token: &str) -> bool {
    super::undo_stack()
        .lock()
        .map(|stack| stack.iter().any(|held| held.token == token))
        .unwrap_or(false)
}

/// Onto the back of the stack rather than the front. A record from last Tuesday must not be what
/// `z` reaches for ahead of whatever somebody just did, and a full stack is a session busy enough
/// that the offer can wait for the next startup.
fn offer_again(entry: super::UndoEntry) {
    let Ok(mut stack) = super::undo_stack().lock() else {
        return;
    };
    if stack.len() >= super::UNDO_DEPTH || stack.iter().any(|held| held.token == entry.token) {
        return;
    }
    stack.push_back(entry);
}

fn label_for(threads: usize) -> String {
    match threads {
        0 => "Nothing older to mark".to_string(),
        1 => "Marked one older thread as seen".to_string(),
        many => format!("Marked {many} older threads as seen"),
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn start_fresh(
    app: tauri::AppHandle,
    account_id: String,
    older_than_ms: i64,
) -> Result<Undo, String> {
    let db = super::db_of(&app)?;
    let undo = db.with(&account_id, |conn| {
        start(conn, &account_id, older_than_ms, write::now_ms())
    })?;
    crate::emit_store_changed(&app, "threads");
    Ok(undo)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rusqlite::Connection;
    use tauri::async_runtime::block_on;

    use crate::db;
    use crate::provider::fake::FakeProvider;
    use crate::sync::{hydrate, Store};

    use super::*;

    struct Memory(Mutex<Connection>);

    impl Store for Memory {
        fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
            let conn = self.0.lock().map_err(|e| e.to_string())?;
            f(&conn)
        }
    }

    impl Memory {
        fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> T {
            self.with(f).expect("the mirror")
        }
    }

    fn store() -> Memory {
        Memory(Mutex::new(db::memory().expect("a pair of in-memory databases")))
    }

    fn eml(id: &str, subject: &str, at_ms: i64) -> Vec<u8> {
        format!(
            "Message-ID: <{id}@example.test>\r\nFrom: Ana <ana@example.test>\r\n\
             To: You <you@example.test>\r\nSubject: {subject}\r\nDate: {}\r\n\r\nBody.\r\n",
            chrono::DateTime::from_timestamp_millis(at_ms)
                .expect("a moment inside the epoch")
                .to_rfc2822()
        )
        .into_bytes()
    }

    fn days_ago(days: i64) -> i64 {
        write::now_ms() - days * write::DAY_MS
    }

    /// Fills the mirror through the ordinary hydration path, so the rows are the rows a sync would
    /// have left behind.
    fn mailbox(store: &Memory, fake: &FakeProvider, mail: &[(&str, &str, &[&str], i64)]) {
        let mut ids = Vec::new();
        for (id, thread, labels, at) in mail {
            fake.add_eml(id, thread, labels, &eml(id, id, *at));
            ids.push(id.to_string());
        }
        block_on(hydrate::headers(store, fake, &ids, false)).expect("the metadata");
    }

    fn seen(store: &Memory, id: &str) -> bool {
        store.read(|conn| {
            conn.query_row("SELECT seen FROM messages WHERE id = ?1", [id], |row| {
                row.get::<_, i64>(0)
            })
            .map(|seen| seen != 0)
            .map_err(|e| e.to_string())
        })
    }

    #[test]
    fn start_fresh_marks_the_old_unseen_threads_and_only_those() {
        let store = store();
        let fake = FakeProvider::new();
        mailbox(
            &store,
            &fake,
            &[
                ("old", "t1", &["INBOX", "UNREAD"], days_ago(30)),
                ("older", "t2", &["INBOX", "UNREAD"], days_ago(60)),
                ("recent", "t3", &["INBOX", "UNREAD"], days_ago(1)),
                ("read", "t4", &["INBOX"], days_ago(30)),
                ("binned", "t5", &["TRASH", "UNREAD"], days_ago(30)),
            ],
        );

        let undo = store.read(|conn| start(conn, "acct", days_ago(7), write::now_ms()));
        assert_eq!(undo.label, "Marked 2 older threads as seen");

        assert!(seen(&store, "old"));
        assert!(seen(&store, "older"));
        assert!(!seen(&store, "recent"), "inside the age it was left alone");
        assert!(!seen(&store, "binned"), "the bin is not New for you");

        // One push, carrying exactly the messages that changed.
        let queued = store.read(|conn| write::outbox_rows(conn, 10));
        assert_eq!(queued.len(), 1);
        let payload: outbox::FlagsPayload =
            serde_json::from_str(&queued[0].payload).expect("a flags payload");
        assert_eq!(payload.ids, vec!["old".to_string(), "older".to_string()]);
        assert_eq!(payload.patch.seen, Some(true));
    }

    #[test]
    fn the_reversal_puts_back_exactly_what_was_unseen() {
        let store = store();
        let fake = FakeProvider::new();
        mailbox(
            &store,
            &fake,
            &[
                ("unread", "t1", &["INBOX", "UNREAD"], days_ago(30)),
                ("already-read", "t1", &["INBOX"], days_ago(31)),
                ("recent", "t2", &["INBOX", "UNREAD"], days_ago(1)),
            ],
        );

        let undo = store.read(|conn| start(conn, "acct", days_ago(7), write::now_ms()));
        assert!(seen(&store, "unread"));

        let restored = store.read(|conn| reverse(conn, &undo.token));
        assert_eq!(restored, 1);
        assert!(!seen(&store, "unread"));
        assert!(
            seen(&store, "already-read"),
            "a message that was already seen is not made unseen by taking this back"
        );
        assert!(!seen(&store, "recent"));

        assert!(
            store.read(|conn| pending(conn, write::now_ms())).is_empty(),
            "and the offer is spent"
        );
    }

    #[test]
    fn a_reversal_still_works_after_a_restart_because_the_record_is_on_disk() {
        let store = store();
        let fake = FakeProvider::new();
        mailbox(&store, &fake, &[("old", "t1", &["INBOX", "UNREAD"], days_ago(30))]);

        let undo = store.read(|conn| start(conn, "acct", days_ago(7), write::now_ms()));
        assert!(seen(&store, "old"));

        // A restart, as far as this promise is concerned: the in-memory stack is gone and the
        // token is all that is left.
        assert!(super::super::undo_take(&undo.token).is_some());
        assert!(super::super::undo_take(&undo.token).is_none());

        assert_eq!(store.read(|conn| reverse(conn, &undo.token)), 1);
        assert!(!seen(&store, "old"));
    }

    #[test]
    fn a_record_older_than_seven_days_is_pruned_rather_than_offered() {
        let store = store();
        let fake = FakeProvider::new();
        mailbox(
            &store,
            &fake,
            &[
                ("old", "t1", &["INBOX", "UNREAD"], days_ago(30)),
                ("newer", "t2", &["INBOX", "UNREAD"], days_ago(20)),
            ],
        );

        let stale = store.read(|conn| start(conn, "acct", days_ago(25), days_ago(8)));
        let fresh = store.read(|conn| start(conn, "acct", days_ago(7), write::now_ms()));
        assert_eq!(store.read(|conn| pending(conn, write::now_ms())).len(), 1);

        let offered: Vec<String> = store
            .read(|conn| pending(conn, write::now_ms()))
            .into_iter()
            .map(|undo| undo.token)
            .collect();
        assert_eq!(offered, vec![fresh.token]);

        let refused = store.with(|conn| reverse(conn, &stale.token));
        assert_eq!(
            refused,
            Err("that change can no longer be taken back".to_string())
        );
        assert!(seen(&store, "old"), "and what it did stands");
    }
}
