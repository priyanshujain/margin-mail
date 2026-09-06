// Opening the two databases and handing out the one connection that sees both.
//
// Each account gets its own pair of files under `accounts/<id>/`: `mirror.db`, which is derived and
// disposable, and `state.db`, which is everything the person decided. They are separate files
// because they have separate lifetimes: clearing the mirror is deleting a file, exporting the
// state is copying one, and the backup uploads segments of the second and never a byte of the
// first.
//
// They are opened on one connection, with the state file attached as `state`, so a view can be one
// SQL statement across both rather than two queries joined in Rust. Every join in `mirror::read`
// depends on that, and it is the reason this module exists rather than each side opening its own.
//
// One connection per account, behind a mutex, reached from `#[tauri::command(async)]` handlers as
// the calendar does. WAL so a long hydration does not block a read, and a busy timeout because
// several accounts in one process on one disk will collide eventually and the alternative is a
// spurious "database is locked" in front of somebody's mail.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::library::app_data_dir;
use crate::{mirror, state};

/// Long enough to outlast a hydration batch's write, short enough that a real deadlock is still a
/// bug that shows up rather than a hang.
const BUSY_TIMEOUT_MS: u32 = 5_000;

pub struct Db {
    root: PathBuf,
    /// Where a removed account's pair goes when the person chose to keep it: `kept/<id>`, beside
    /// `accounts/` rather than inside it, so nothing that reads `on_disk` can take it for a live
    /// mailbox.
    kept: PathBuf,
    connections: Mutex<HashMap<String, Connection>>,
}

impl Db {
    pub fn open(app: &tauri::AppHandle) -> Result<Db, String> {
        let data = app_data_dir(app)?;
        // The sync log lives beside the account directories, and opening the databases is the
        // one moment the engine's own code is handed the directory: the engine itself never sees
        // an app handle, which is what lets it run in a test with nothing behind it.
        crate::log::init(&data);
        let root = data.join("accounts");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        Ok(Db {
            root,
            kept: data.join("kept"),
            connections: Mutex::new(HashMap::new()),
        })
    }

    pub fn account_dir(&self, account_id: &str) -> PathBuf {
        self.root.join(account_id)
    }

    /// Runs a closure against one account's connection. Every read and every write goes through
    /// here, so there is one place that knows a connection might need opening first.
    pub fn with<T>(
        &self,
        account_id: &str,
        f: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut open = self.connections.lock().map_err(|e| e.to_string())?;
        if !open.contains_key(account_id) {
            let dir = self.account_dir(account_id);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            open.insert(account_id.to_string(), connect(&dir)?);
        }
        let conn = open.get(account_id).expect("just inserted");
        f(conn)
    }

    /// Closes the connection so the files can be moved or deleted. Opening again is automatic.
    pub fn close(&self, account_id: &str) -> Result<(), String> {
        let mut open = self.connections.lock().map_err(|e| e.to_string())?;
        open.remove(account_id);
        Ok(())
    }

    /// The accounts that have a database on disk, which is not always the same list as the accounts
    /// that have a token: a removal takes one away before the other.
    pub fn on_disk(&self) -> Vec<String> {
        std::fs::read_dir(&self.root)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.path().is_dir())
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Moves an account's pair out of the live set without deleting it. `on_disk` stops listing
    /// it, so no list, badge or sync pass sees a mailbox nobody is signed in to, and `restore`
    /// brings it back the day the account is added again. An older kept copy of the same account
    /// is replaced: the pair just removed is the one the person was looking at. Call `close`
    /// first, because a connection open on the old path would keep writing there.
    pub fn set_aside(&self, account_id: &str) -> Result<(), String> {
        let live = self.account_dir(account_id);
        if !live.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.kept).map_err(|e| e.to_string())?;
        let kept = self.kept.join(account_id);
        if kept.exists() {
            std::fs::remove_dir_all(&kept).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&live, &kept).map_err(|e| e.to_string())
    }

    /// The other half: an account added again gets the pair that was set aside when it was
    /// removed, decisions and all. Nothing happens when a live pair already exists, because a pair
    /// that has been written to since is the newer truth and a rename over it would lose it.
    pub fn restore(&self, account_id: &str) -> Result<(), String> {
        let kept = self.kept.join(account_id);
        let live = self.account_dir(account_id);
        if !kept.exists() || live.exists() {
            return Ok(());
        }
        std::fs::rename(&kept, &live).map_err(|e| e.to_string())
    }
}

fn connect(dir: &Path) -> Result<Connection, String> {
    let conn = Connection::open(dir.join("mirror.db")).map_err(|e| e.to_string())?;
    prepare(&conn, &dir.join("state.db").to_string_lossy())?;
    Ok(conn)
}

fn prepare(conn: &Connection, state_path: &str) -> Result<(), String> {
    // Not a prepared statement: ATTACH takes a literal, and the path is ours rather than anyone's
    // input. The escaping is still done, because a home directory with an apostrophe in it exists.
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = OFF;
         ATTACH DATABASE '{}' AS state;",
        state_path.replace('\'', "''")
    ))
    .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS as u64))
        .map_err(|e| e.to_string())?;

    mirror::schema::migrate(conn)?;
    state::schema::migrate(conn)
}

/// A pair of in-memory databases with both schemas, for tests. Attaching `:memory:` gives a second,
/// separate in-memory database, so the boundary the real thing has is the boundary a test has.
#[cfg(test)]
pub fn memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch("ATTACH DATABASE ':memory:' AS state;")
        .map_err(|e| e.to_string())?;
    mirror::schema::migrate(&conn)?;
    state::schema::migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_disk(data: &Path) -> Db {
        Db {
            root: data.join("accounts"),
            kept: data.join("kept"),
            connections: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn a_kept_pair_leaves_the_live_set_and_comes_back_on_restore() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = on_disk(tmp.path());
        let dir = db.account_dir("a1");
        std::fs::create_dir_all(&dir).expect("live dir");
        std::fs::write(dir.join("state.db"), b"decisions").expect("a file to keep");

        db.set_aside("a1").expect("set aside");
        assert!(db.on_disk().is_empty(), "a kept account is not a live one");
        assert!(!dir.exists());

        db.restore("a1").expect("restore");
        assert_eq!(db.on_disk(), vec!["a1".to_string()]);
        assert_eq!(std::fs::read(dir.join("state.db")).expect("the file"), b"decisions");
    }

    #[test]
    fn keeping_again_replaces_the_older_copy_and_restore_never_overwrites_a_live_pair() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = on_disk(tmp.path());
        let dir = db.account_dir("a1");

        std::fs::create_dir_all(&dir).expect("live dir");
        std::fs::write(dir.join("state.db"), b"first").expect("write");
        db.set_aside("a1").expect("first keep");

        std::fs::create_dir_all(&dir).expect("live dir again");
        std::fs::write(dir.join("state.db"), b"second").expect("write");
        db.set_aside("a1").expect("second keep");
        assert_eq!(std::fs::read(tmp.path().join("kept/a1/state.db")).expect("kept"), b"second");

        std::fs::create_dir_all(&dir).expect("a live pair in the way");
        std::fs::write(dir.join("state.db"), b"live").expect("write");
        db.restore("a1").expect("restore is a no-op here");
        assert_eq!(std::fs::read(dir.join("state.db")).expect("live"), b"live");
        assert!(tmp.path().join("kept/a1").exists(), "the kept copy is not thrown away either");

        // Nothing to do when nothing was kept.
        db.set_aside("nobody").expect("no live dir is fine");
        db.restore("nobody").expect("no kept dir is fine");
    }

    #[test]
    fn a_fresh_pair_has_both_schemas_and_one_connection_sees_both() {
        let conn = memory().expect("open");

        conn.execute(
            "INSERT INTO threads (provider_thread_id, thread_key, latest_ms) VALUES ('t1', 'k1', 10)",
            [],
        )
        .expect("mirror write");
        conn.execute(
            "INSERT INTO state.piles (thread_key, pile) VALUES ('k1', 'reply-later')",
            [],
        )
        .expect("state write");

        let pile: String = conn
            .query_row(
                "SELECT p.pile FROM threads t JOIN state.piles p ON p.thread_key = t.thread_key",
                [],
                |row| row.get(0),
            )
            .expect("the join across both databases");
        assert_eq!(pile, "reply-later");
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let conn = memory().expect("open");
        mirror::schema::migrate(&conn).expect("mirror again");
        state::schema::migrate(&conn).expect("state again");
        assert_eq!(mirror::schema::version(&conn).expect("version"), mirror::schema::VERSION);
    }
}
