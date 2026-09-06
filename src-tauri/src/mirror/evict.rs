// The window, enforced.
//
// The mirror holds every thread whose latest message falls inside the account's window, and on top
// of that, at any age, every thread that carries something a person decided, every starred thread,
// every draft, and anything waiting in the outbox. Eviction removes the rest. It never touches the
// state database: nothing anybody decided is ever evicted, and a state row pointing at a thread
// that is no longer mirrored is the expected shape rather than a leak.
//
// Widening a window is not eviction. It queues a backfill, which fills the newly covered range
// through the same hydration path as a first sync, so there is one way to fill the mirror.

use rusqlite::Connection;

use super::write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChange {
    Same,
    Shrunk,
    Widened,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub threads: u32,
    pub messages: u32,
}

pub const LAST_EVICT_KEY: &str = "last-evict";

/// Everything that keeps a thread whatever its age. Written once and used by both the victim
/// query and anything else that needs to ask "would this survive".
const KEPT: &str = "\
       t.starred = 1
    OR t.has_draft = 1
    OR EXISTS (SELECT 1 FROM state.piles k WHERE k.thread_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM state.snoozes k WHERE k.thread_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM state.returned k WHERE k.thread_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM state.notes k WHERE k.thread_key = t.thread_key AND k.deleted = 0)
    OR EXISTS (SELECT 1 FROM state.renames k WHERE k.thread_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM state.merges k
               WHERE k.thread_key = t.thread_key OR k.merged_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM state.thread_flags k
               WHERE k.thread_key = t.thread_key AND (k.ignored = 1 OR k.notify = 1))
    OR EXISTS (SELECT 1 FROM state.clips k WHERE k.thread_key = t.thread_key AND k.deleted = 0)
    OR EXISTS (SELECT 1 FROM outbox o WHERE o.thread_key = t.thread_key)
    OR EXISTS (SELECT 1 FROM drafts d WHERE d.thread_key = t.thread_key)";

/// Runs a pass. `cutoff` of None is an account keeping everything, where the only thing eviction
/// still has to do is take away the transient rows a provider search pulled in.
pub fn run(conn: &Connection, now: i64) -> Result<Report, String> {
    let cutoff = write::window_start(conn, now)?;
    let outside = match cutoff {
        Some(cutoff) => format!("(t.latest_ms < {cutoff} OR t.transient = 1)"),
        None => "t.transient = 1".to_string(),
    };

    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS evicting (tid TEXT PRIMARY KEY);
         DELETE FROM evicting;",
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        &format!(
            "INSERT INTO evicting (tid)
             SELECT t.provider_thread_id FROM threads t
             WHERE {outside} AND NOT ({KEPT})"
        ),
        [],
    )
    .map_err(|e| e.to_string())?;

    let messages = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE provider_thread_id IN (SELECT tid FROM evicting)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())?;

    // Order matters: the index and the bodies hang off message ids, so they go before the messages
    // do. Nothing here declares a foreign key, which is why the order is written down.
    for sql in [
        "DELETE FROM search WHERE message_id IN
            (SELECT id FROM messages WHERE provider_thread_id IN (SELECT tid FROM evicting))",
        "DELETE FROM bodies WHERE message_id IN
            (SELECT id FROM messages WHERE provider_thread_id IN (SELECT tid FROM evicting))",
        "DELETE FROM attachments WHERE message_id IN
            (SELECT id FROM messages WHERE provider_thread_id IN (SELECT tid FROM evicting))",
        "DELETE FROM messages WHERE provider_thread_id IN (SELECT tid FROM evicting)",
    ] {
        conn.execute(sql, []).map_err(|e| e.to_string())?;
    }

    let threads = conn
        .execute(
            "DELETE FROM threads WHERE provider_thread_id IN (SELECT tid FROM evicting)",
            [],
        )
        .map_err(|e| e.to_string())?;

    conn.execute_batch("DELETE FROM evicting;")
        .map_err(|e| e.to_string())?;
    write::meta_set(conn, LAST_EVICT_KEY, &now.to_string())?;

    Ok(Report {
        threads: threads as u32,
        messages: messages as u32,
    })
}

/// Once a day, and again whenever the window shrinks.
pub fn due(conn: &Connection, now: i64) -> Result<bool, String> {
    let last = write::meta_i64(conn, LAST_EVICT_KEY)?.unwrap_or(0);
    Ok(now - last >= write::DAY_MS)
}

/// Records a new window and says what it means. Shrinking asks for a pass; widening queues a
/// backfill over the range the mirror does not hold yet.
pub fn set_window(conn: &Connection, days: i64, now: i64) -> Result<WindowChange, String> {
    let before = write::window_days(conn)?;
    write::meta_set(conn, write::WINDOW_KEY, &days.to_string())?;
    if days == before {
        return Ok(WindowChange::Same);
    }
    // Zero is everything, so it is wider than any number of days and narrower than none of them.
    let wider = days == 0 || (before != 0 && days > before);
    if wider {
        let target = (days > 0).then(|| now - days * write::DAY_MS).unwrap_or(0);
        write::meta_set(conn, write::BACKFILL_KEY, &target.to_string())?;
        return Ok(WindowChange::Widened);
    }
    Ok(WindowChange::Shrunk)
}

/// The moment a queued backfill reaches back to, or None when there is nothing to fill.
pub fn backfill_target(conn: &Connection) -> Result<Option<i64>, String> {
    write::meta_i64(conn, write::BACKFILL_KEY)
}

pub fn backfill_done(conn: &Connection) -> Result<(), String> {
    write::meta_clear(conn, write::BACKFILL_KEY)
}
