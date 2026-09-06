// Tables and migrations for the mirror. Forward only and idempotent: it runs the steps between the
// version recorded in `meta` and VERSION, and refuses to open a database written by a newer build
// rather than silently misreading it. A downgrade that half-understands a schema is how mail goes
// missing, and a clear refusal is recoverable.

use rusqlite::{Connection, OptionalExtension};

pub const VERSION: i32 = 2;

const VERSION_KEY: &str = "schema_version";

const V1: &str = include_str!("mirror.sql");

/// Which surface a body reads on, from `sanitize::surface`. A column rather than something worked
/// out on open, because a thread opens out of the mirror and a DOM walk per message is exactly the
/// cost that was taken out of an open. Every row already here defaults to the theme and is decided
/// properly the first time its thread is opened, which the `RENDER_VERSION` bump alongside this
/// makes happen with no re-download.
const V2: &str = "ALTER TABLE bodies ADD COLUMN surface TEXT NOT NULL DEFAULT 'theme';";

pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    let found = version(conn)?;
    if found > VERSION {
        return Err(format!(
            "this mailbox is at schema {found}, which is newer than this build understands ({VERSION})"
        ));
    }
    if found == VERSION {
        return Ok(());
    }

    conn.execute_batch("BEGIN;").map_err(|e| e.to_string())?;
    let stepped = (|| -> Result<(), String> {
        if found < 1 {
            conn.execute_batch(V1).map_err(|e| e.to_string())?;
        }
        if found < 2 {
            conn.execute_batch(V2).map_err(|e| e.to_string())?;
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![VERSION_KEY, VERSION.to_string()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();

    match stepped {
        Ok(()) => conn.execute_batch("COMMIT;").map_err(|e| e.to_string()),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(e);
        }
    }?;

    unescape_snippets(conn)
}

const UNESCAPED_KEY: &str = "snippets-unescaped";

/// Snippets Gmail sent escaped, put back as the text they read as.
///
/// Gmail returns the `snippet` field HTML escaped, so an apostrophe arrives as `&#39;`, and the
/// row that showed `I&#39;m` beside a subject reading `I'm` had carried it since the day it was
/// written. `gmail::headers_from` decodes it now, but Gmail is not going to send that metadata
/// again for mail already in the mirror, so without this the fix would only ever reach new mail.
///
/// A repair rather than a migration, which is why it hangs off a key of its own rather than the
/// schema version. Nothing about the tables changed, and bumping the version would make this build
/// refuse to open a mailbox the previous one had touched, over a preview line.
fn unescape_snippets(conn: &Connection) -> Result<(), String> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [UNESCAPED_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if done.is_some() {
        return Ok(());
    }

    // Only rows that could hold a reference. An ampersand with a semicolon after it is a loose
    // test, and it is meant to be: the decoder leaves anything that is not a reference alone, so
    // the cost of a false positive is one wasted write and the cost of a false negative is a row
    // that stays wrong for ever.
    for table in ["messages", "threads"] {
        let sql = format!("SELECT rowid, snippet FROM {table} WHERE snippet LIKE '%&%;%'");
        let rows = {
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let found = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            found
        };
        let update = format!("UPDATE {table} SET snippet = ?2 WHERE rowid = ?1");
        for (rowid, snippet) in rows {
            let decoded = crate::sanitize::decode_entities(&snippet);
            if decoded == snippet {
                continue;
            }
            conn.execute(&update, rusqlite::params![rowid, decoded])
                .map_err(|e| e.to_string())?;
        }
    }

    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [UNESCAPED_KEY],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn version(conn: &Connection) -> Result<i32, String> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [VERSION_KEY], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(raw.and_then(|v| v.parse::<i32>().ok()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailbox() -> Connection {
        let conn = Connection::open_in_memory().expect("memory");
        migrate(&conn).expect("migrate");
        conn
    }

    /// A mailbox written by the build before this one, opened by this one.
    ///
    /// Two things have to be true afterwards and neither is obvious from the migration alone. The
    /// column has to be there with every existing row on the safe default, and the bodies already
    /// cached have to come back as work rather than as answers: their stamp is the old
    /// `RENDER_VERSION`, so `stale_renders` is what turns a schema step into a mailbox that has
    /// actually been re-decided, and it does it on the next open with nothing re-downloaded.
    #[test]
    fn a_mailbox_from_the_build_before_this_one_gains_the_column_and_re_renders_what_it_had() {
        let conn = Connection::open_in_memory().expect("memory");
        conn.execute_batch("ATTACH DATABASE ':memory:' AS state;")
            .expect("attach");
        conn.execute_batch(V1).expect("the schema as version 1 shipped it");
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '1')",
            [VERSION_KEY],
        )
        .expect("stamp");
        crate::state::schema::migrate(&conn).expect("state");

        conn.execute_batch(
            "INSERT INTO threads (provider_thread_id, thread_key) VALUES ('p1', 'k1');
             INSERT INTO messages (id, provider_thread_id, thread_key, hydrated)
                 VALUES ('m1', 'p1', 'k1', 1);
             INSERT INTO bodies (message_id, raw, html, is_html, render_version)
                 VALUES ('m1', x'00', '<p>Hi</p>', 1, 1);",
        )
        .expect("a mailbox with a body in it");

        migrate(&conn).expect("migrate");

        assert_eq!(version(&conn).expect("version"), VERSION);
        let surface: String = conn
            .query_row("SELECT surface FROM bodies WHERE message_id = 'm1'", [], |r| r.get(0))
            .expect("the new column");
        assert_eq!(surface, "theme");

        let stale = super::super::read::stale_renders(&conn, "k1").expect("stale");
        assert_eq!(stale, vec!["m1".to_string()]);
    }

    #[test]
    fn a_mailbox_from_a_newer_build_is_refused_rather_than_half_read() {
        let conn = Connection::open_in_memory().expect("memory");
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta (key, value) VALUES ('schema_version', '3');",
        )
        .expect("a mailbox from the future");
        assert!(migrate(&conn).is_err());
    }

    #[test]
    fn a_snippet_written_before_the_decoder_existed_is_repaired_on_the_next_open() {
        let conn = mailbox();
        conn.execute(
            "INSERT INTO threads (provider_thread_id, thread_key, subject, snippet)
             VALUES ('p1', 't1', 'The lease', 'I&#39;m attaching it &amp; signing')",
            [],
        )
        .expect("insert");
        // The repair only runs once, so the flag it left has to come off for it to run again.
        conn.execute("DELETE FROM meta WHERE key = ?1", [UNESCAPED_KEY])
            .expect("clear");

        unescape_snippets(&conn).expect("repair");

        let snippet: String = conn
            .query_row("SELECT snippet FROM threads WHERE thread_key = 't1'", [], |r| r.get(0))
            .expect("read");
        assert_eq!(snippet, "I'm attaching it & signing");
    }

    #[test]
    fn the_repair_runs_once_and_then_leaves_the_mailbox_alone() {
        let conn = mailbox();
        // Set by the first migrate, so a snippet arriving later is the decoder's job and not this
        // one's. Repairing on every open would rewrite an ampersand a sender really did send.
        conn.execute(
            "INSERT INTO threads (provider_thread_id, thread_key, subject, snippet)
             VALUES ('p1', 't1', 'Bench', 'Tom &amp; Jerry')",
            [],
        )
        .expect("insert");

        unescape_snippets(&conn).expect("repair");

        let snippet: String = conn
            .query_row("SELECT snippet FROM threads WHERE thread_key = 't1'", [], |r| r.get(0))
            .expect("read");
        assert_eq!(snippet, "Tom &amp; Jerry");
    }
}
