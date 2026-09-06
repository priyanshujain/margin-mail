// Tables and migrations for the state database, which is attached as `state`, so every
// statement here names it.
//
// Tables and migrations for the state database. Forward only and idempotent: it runs the steps between the
// version recorded in `state.meta` and VERSION, and refuses to open a database written by a newer build
// rather than silently misreading it. A downgrade that half-understands a schema is how mail goes
// missing, and a clear refusal is recoverable.

use rusqlite::{Connection, OptionalExtension};

pub const VERSION: i32 = 1;

const VERSION_KEY: &str = "schema_version";

const V1: &str = include_str!("state.sql");

pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state.meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    let found = version(conn)?;
    if found > VERSION {
        return Err(format!(
            "these decisions are at schema {found}, which is newer than this build understands ({VERSION})"
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
        conn.execute(
            "INSERT INTO state.meta (key, value) VALUES (?1, ?2)
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
            Err(e)
        }
    }
}

pub fn version(conn: &Connection) -> Result<i32, String> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM state.meta WHERE key = ?1", [VERSION_KEY], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(raw.and_then(|v| v.parse::<i32>().ok()).unwrap_or(0))
}
