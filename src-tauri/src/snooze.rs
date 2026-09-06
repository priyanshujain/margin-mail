// Snooze, and the evaluation that brings a thread back.
//
// Nothing here runs in the background and nothing fires at an exact time. A device that opens the
// app, comes to the foreground or wakes asks `snooze_evaluate` what is due, and that call is the
// only moment a snooze can spend itself. That is the whole design: a mail client that promises to
// interrupt you at 08:00 has to be running at 08:00, and this one does not have to be.
//
// A thread that comes back lands in the Back group at the top of its list and stays there until it
// is opened. The moment it was due survives the return in `state.returned.due_ms`, so a thread that
// comes back on Tuesday for a Monday reminder says "Due yesterday" rather than pretending it is
// new.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Weekday};
use rusqlite::{Connection, OptionalExtension};

use crate::decisions::{db_of, holders, plural};
use crate::dto::{SnoozeKind, SnoozeTimes, Undo};
use crate::mirror::write::now_ms;
use crate::state::{self, read::SnoozeRow};
use crate::sync;
use crate::undo::Stack;

static UNDO: Stack<UndoSnoozes> = Stack::new("snooze");

// ---------------------------------------------------------------------------------------------
// When a snooze comes back
// ---------------------------------------------------------------------------------------------

fn at_local(day: NaiveDate, minutes: u32, tz: &FixedOffset) -> Option<i64> {
    let time = NaiveTime::from_num_seconds_from_midnight_opt(minutes.min(1_439) * 60, 0)?;
    tz.from_local_datetime(&day.and_time(time))
        .single()
        .map(|moment| moment.timestamp_millis())
}

/// The next day of the week named, at the time named, that has not already passed. Saturday morning
/// asking for "this weekend" means this afternoon; Saturday evening means the one after.
fn coming(
    from: NaiveDate,
    weekday: Weekday,
    minutes: u32,
    now_ms: i64,
    tz: &FixedOffset,
) -> Option<i64> {
    let mut day = from;
    for _ in 0..8 {
        if day.weekday() == weekday {
            if let Some(at) = at_local(day, minutes, tz) {
                if at > now_ms {
                    return Some(at);
                }
            }
        }
        day = day.succ_opt()?;
    }
    None
}

/// What each item on the picker means, in epoch milliseconds.
///
/// The offset is passed in rather than read here so this is a pure function of its arguments: the
/// times are local ones, "tomorrow at 8" is a promise about somebody's morning, and a test has to
/// be able to put the machine anywhere without the answer moving.
///
/// `Date` and `IfNoReply` both name their own moment on the picker; what comes back here is the
/// moment the picker opens on, which is tomorrow.
pub fn return_at(kind: SnoozeKind, now_ms: i64, times: &SnoozeTimes, offset_minutes: i32) -> i64 {
    let Some(tz) = FixedOffset::east_opt(offset_minutes * 60) else {
        return now_ms;
    };
    let Some(now) = DateTime::from_timestamp_millis(now_ms) else {
        return now_ms;
    };
    let today = now.with_timezone(&tz).date_naive();
    let tomorrow = || {
        today
            .succ_opt()
            .and_then(|day| at_local(day, times.tomorrow_at, &tz))
    };
    match kind {
        SnoozeKind::LaterToday => Some(now_ms + times.later_today_hours as i64 * 3_600_000),
        SnoozeKind::Tomorrow | SnoozeKind::Date | SnoozeKind::IfNoReply => tomorrow(),
        SnoozeKind::Weekend => coming(today, Weekday::Sat, times.weekend_at, now_ms, &tz),
        SnoozeKind::NextWeek => coming(today, Weekday::Mon, times.next_week_at, now_ms, &tz),
    }
    .unwrap_or(now_ms)
}

/// This machine's offset from UTC right now, in minutes east.
pub fn local_offset_minutes() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

// ---------------------------------------------------------------------------------------------
// The state a snooze is set against and read back from
// ---------------------------------------------------------------------------------------------

/// The latest message in the thread when the snooze is set. For `if-no-reply` this is the line a
/// reply has to arrive after to count as one.
fn watermark(conn: &Connection, thread_key: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(m.date_ms), 0) FROM messages m
          WHERE m.provider_thread_id IN (
              SELECT provider_thread_id FROM threads
               WHERE thread_key = ?1
                  OR thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1))",
        [thread_key],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Whether anybody but this account has written in the thread since. A follow up of your own is not
/// a reply to yourself, which is the whole point of the reminder.
fn replied_since(conn: &Connection, thread_key: &str, watermark: i64) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages m
              WHERE m.date_ms > ?2 AND m.sent = 0 AND m.draft = 0
                AND m.provider_thread_id IN (
                    SELECT provider_thread_id FROM threads
                     WHERE thread_key = ?1
                        OR thread_key IN (
                            SELECT thread_key FROM state.merges WHERE merged_key = ?1))",
            rusqlite::params![thread_key, watermark],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

/// When a thread waiting in Back was due. `state::read` hands back the keys and not this, because
/// the only two callers are the line that says "Due yesterday" and a reversal.
pub fn returned_due(conn: &Connection, thread_key: &str) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT due_ms FROM state.returned WHERE thread_key = ?1",
        [thread_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Setting, clearing, evaluating
// ---------------------------------------------------------------------------------------------

/// What a key was before a snooze touched it, which is both halves: a thread can be snoozed while
/// it is sitting in Back from the last one.
pub struct Prior {
    pub thread_key: String,
    pub snooze: Option<SnoozeRow>,
    pub returned_due: Option<i64>,
}

fn prior(conn: &Connection, thread_key: &str) -> Result<Prior, String> {
    Ok(Prior {
        thread_key: thread_key.to_string(),
        snooze: state::read::snooze_of(conn, thread_key)?,
        returned_due: returned_due(conn, thread_key)?,
    })
}

pub fn set(
    conn: &Connection,
    keys: &[String],
    kind: SnoozeKind,
    return_at_ms: i64,
) -> Result<Vec<Prior>, String> {
    let mut before = Vec::new();
    for key in keys {
        before.push(prior(conn, key)?);
        let watermark = watermark(conn, key)?;
        state::write::set_snooze(conn, key, return_at_ms, kind, watermark)?;
        // Snoozing a thread that is sitting in Back sends it away again, so it is no longer waiting
        // to be opened.
        state::write::clear_returned(conn, key)?;
    }
    Ok(before)
}

pub fn clear(conn: &Connection, keys: &[String]) -> Result<Vec<Prior>, String> {
    let mut before = Vec::new();
    for key in keys {
        let held = prior(conn, key)?;
        if held.snooze.is_some() {
            state::write::clear_snooze(conn, key)?;
        }
        if held.returned_due.is_some() {
            state::write::clear_returned(conn, key)?;
        }
        before.push(held);
    }
    Ok(before)
}

/// Every snooze whose moment has passed, brought back, and every `if-no-reply` somebody has
/// answered, cancelled. Returns the threads that came back, in the order they were due.
///
/// A snooze that has spent itself is cleared rather than kept, and the moment it was due is written
/// onto the `returned` row, which is the one thing that has to outlive it.
pub fn evaluate(conn: &Connection, now: i64) -> Result<Vec<String>, String> {
    let mut back = Vec::new();
    for row in state::read::snoozes(conn)? {
        if row.kind == SnoozeKind::IfNoReply && replied_since(conn, &row.thread_key, row.watermark)?
        {
            // The reply is the answer the reminder was waiting for, and it lands as normal.
            state::write::clear_snooze(conn, &row.thread_key)?;
            continue;
        }
        if row.return_at > now {
            continue;
        }
        if returned_due(conn, &row.thread_key)?.is_some() {
            continue;
        }
        state::write::mark_returned(conn, &row.thread_key, row.return_at)?;
        state::write::clear_snooze(conn, &row.thread_key)?;
        back.push(row.thread_key);
    }
    Ok(back)
}

/// Reading a thread that came back takes it out of Back. It is the one thing that does, which is
/// what "stays in Back until opened" means.
pub fn opened(conn: &Connection, thread_key: &str) -> Result<(), String> {
    if returned_due(conn, thread_key)?.is_none() {
        return Ok(());
    }
    state::write::clear_returned(conn, thread_key)
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

pub struct UndoSnoozes {
    pub before: Vec<(String, Vec<Prior>)>,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that change can no longer be taken back")?;
    let db = db_of(app)?;
    for (account_id, before) in &entry.before {
        db.with(account_id, |conn| {
            for held in before {
                restore(conn, held)?;
            }
            Ok(())
        })?;
    }
    crate::emit_store_changed(app, "threads state");
    Ok(())
}

fn restore(conn: &Connection, held: &Prior) -> Result<(), String> {
    match &held.snooze {
        Some(row) => state::write::set_snooze(
            conn,
            &held.thread_key,
            row.return_at,
            row.kind,
            row.watermark,
        )?,
        None => state::write::clear_snooze(conn, &held.thread_key)?,
    }
    match held.returned_due {
        Some(due) => state::write::mark_returned(conn, &held.thread_key, due),
        None => state::write::clear_returned(conn, &held.thread_key),
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn snooze_set(
    app: tauri::AppHandle,
    keys: Vec<String>,
    kind: SnoozeKind,
    return_at_ms: i64,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    // The picker names the moment, so the number arrives with the call. A caller that has no moment
    // to give gets the one this kind means, rather than a snooze that comes back at the epoch.
    let return_at_ms = if return_at_ms > 0 {
        return_at_ms
    } else {
        let times = crate::settings::load(&app)
            .map(|settings| settings.snooze_times)
            .unwrap_or_else(|_| crate::settings::defaults().snooze_times);
        return_at(kind, now_ms(), &times, local_offset_minutes())
    };

    let mut before = Vec::new();
    let mut count = 0usize;
    for (account_id, held) in holders(db.inner(), &keys)? {
        let prior = db.with(&account_id, |conn| set(conn, &held, kind, return_at_ms))?;
        count += prior.len();
        before.push((account_id, prior));
    }

    let label = plural(count, "Snoozed");
    let token = UNDO.push(UndoSnoozes { before }, &label);
    crate::emit_store_changed(&app, "threads state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

#[tauri::command(async)]
pub fn snooze_clear(app: tauri::AppHandle, keys: Vec<String>) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let mut before = Vec::new();
    let mut count = 0usize;
    for (account_id, held) in holders(db.inner(), &keys)? {
        let prior = db.with(&account_id, |conn| clear(conn, &held))?;
        count += prior.len();
        before.push((account_id, prior));
    }

    let label = plural(count, "Snooze cancelled");
    let token = UNDO.push(UndoSnoozes { before }, &label);
    crate::emit_store_changed(&app, "threads state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

/// Runs on open, on foreground and on wake. Returns the threads that came back, so the caller can
/// say how many without asking the list a second time.
#[tauri::command(async)]
pub fn snooze_evaluate(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let db = db_of(&app)?;
    let now = now_ms();
    let mut back = Vec::new();
    for (account_id, _) in sync::accounts(db.inner(), None) {
        back.extend(db.with(&account_id, |conn| evaluate(conn, now))?);
    }
    if !back.is_empty() {
        crate::emit_store_changed(&app, "threads state");
    }
    Ok(back)
}

#[cfg(test)]
mod tests;
