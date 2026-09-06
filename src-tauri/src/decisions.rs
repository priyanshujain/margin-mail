// The decisions that are about a thread rather than about a sender: the note, the name, the merge,
// the ignore and the notify switch.
//
// Every one of them is a thin command over a function in `state::write` that already exists and
// already journals, which is what makes them roam. What is here and not there is the part that
// needs an app: which account's state file the decision belongs in, what the toast says, and how to
// put it back.
//
// The two lookups at the top are shared with `piles`, `snooze` and `clips`. They are here because a
// pile and a snooze are decisions about a thread too, and every one of the five command modules
// asks the same two questions before it can write anything.

use rusqlite::{Connection, OptionalExtension};
use tauri::Manager;

use crate::db::Db;
use crate::dto::{Note, Undo};
use crate::state::{self, journal::Payload, read::ThreadFlags};
use crate::sync;
use crate::undo::Stack;

static UNDO: Stack<UndoDecision> = Stack::new("decision");

// ---------------------------------------------------------------------------------------------
// Which account a decision belongs in
// ---------------------------------------------------------------------------------------------

pub(crate) fn db_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, Db>, String> {
    app.try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())
}

/// Which of these keys one account holds. A merged thread answers to its own key, so a source key
/// finds its account through the merge as well as through the mirror.
fn held_keys(conn: &Connection, keys: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for key in keys {
        let held: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads
                  WHERE thread_key = ?1
                     OR thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1)",
                [key],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if held > 0 {
            out.push(key.clone());
        }
    }
    Ok(out)
}

/// The keys grouped by the account that holds them, so a decision is written into the state file
/// beside the mail it is about and into no other. A key claimed by one account is not offered to
/// the next, because a thread lives in one mailbox and a second row for it would roam as a second
/// decision.
pub(crate) fn holders(db: &Db, keys: &[String]) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for (account_id, _) in sync::accounts(db, None) {
        let wanted: Vec<String> = keys
            .iter()
            .filter(|key| !claimed.contains(key))
            .cloned()
            .collect();
        if wanted.is_empty() {
            break;
        }
        let mine = db.with(&account_id, |conn| held_keys(conn, &wanted))?;
        if mine.is_empty() {
            continue;
        }
        claimed.extend(mine.iter().cloned());
        out.push((account_id, mine));
    }
    Ok(out)
}

/// The account whose state file holds a row, for the commands the frontend names by id alone.
/// `attachments` asks the mirror the same question the same way.
pub(crate) fn account_holding(db: &Db, sql: &str, id: &str) -> Result<String, String> {
    for (account_id, _) in sync::accounts(db, None) {
        let held = db.with(&account_id, |conn| {
            conn.query_row(sql, [id], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        })?;
        if held > 0 {
            return Ok(account_id);
        }
    }
    Err("that is not on this device".to_string())
}

pub(crate) fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        one.to_string()
    } else {
        format!("{one} {count} threads")
    }
}

// ---------------------------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------------------------

/// The message that is latest in the thread right now, which is where the pane puts a new note.
fn latest_message(conn: &Connection, thread_key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT m.id FROM messages m
          WHERE m.provider_thread_id IN (
              SELECT provider_thread_id FROM threads
               WHERE thread_key = ?1
                  OR thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1))
          ORDER BY m.date_ms DESC, m.id DESC LIMIT 1",
        [thread_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// One note by its id, deleted or not, which is what a reversal needs and what `state::read` does
/// not answer: a note is read by its thread everywhere else.
fn note_row(conn: &Connection, id: &str) -> Result<Option<(Note, bool)>, String> {
    conn.query_row(
        "SELECT id, thread_key, body, created_at, after_message_id, deleted FROM state.notes
          WHERE id = ?1",
        [id],
        |row| {
            Ok((
                Note {
                    id: row.get(0)?,
                    thread_key: row.get(1)?,
                    body: row.get(2)?,
                    created_at_ms: row.get(3)?,
                    after_message_id: row.get(4)?,
                },
                row.get::<_, i64>(5)? != 0,
            ))
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn add_note(conn: &Connection, thread_key: &str, body: &str) -> Result<Note, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("a note needs something in it".into());
    }
    let after = latest_message(conn, thread_key)?;
    let id = state::write::add_note(conn, thread_key, body, after.as_deref())?;
    note_row(conn, &id)?
        .map(|(note, _)| note)
        .ok_or_else(|| "that note did not land".to_string())
}

/// Puts a deleted note back. There is no un-delete in `state::write` because nothing but a reversal
/// wants one, and deletion is a flag rather than a missing row precisely so this is possible.
fn restore_note(conn: &Connection, note: &Note) -> Result<(), String> {
    state::journal::append(
        conn,
        &note.id,
        &Payload::Note {
            thread_key: note.thread_key.clone(),
            body: note.body.clone(),
            after_message_id: note.after_message_id.clone(),
            created_at: note.created_at_ms,
            deleted: false,
        },
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Renames and merges
// ---------------------------------------------------------------------------------------------

fn subject_of(conn: &Connection, thread_key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT subject FROM threads WHERE thread_key = ?1 ORDER BY latest_ms DESC LIMIT 1",
        [thread_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// A merge with no name typed takes the longest of the subjects it swallowed, which is section 10's
/// rule and is usually the one that says the most about the conversation.
fn longest_subject(conn: &Connection, keys: &[String]) -> Result<Option<String>, String> {
    let mut best: Option<String> = None;
    for key in keys {
        let Some(subject) = subject_of(conn, key)? else {
            continue;
        };
        let longer = best
            .as_ref()
            .map(|held| subject.chars().count() > held.chars().count())
            .unwrap_or(true);
        if longer {
            best = Some(subject);
        }
    }
    Ok(best)
}

/// The threads pointing directly at this one. `state::read::merge_sources` walks the whole chain,
/// which is the right answer for a view and the wrong one for a reversal: putting a chain back
/// means putting each of its links back, not flattening it.
fn direct_sources(conn: &Connection, merged_key: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT thread_key FROM state.merges WHERE merged_key = ?1 ORDER BY thread_key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([merged_key], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Takes one source back out of a merge. `state::write::unmerge` breaks the whole thread apart,
/// which is what the banner offers; taking back a merge of three into a thread that already held
/// two is not that.
fn unmerge_one(conn: &Connection, source: &str) -> Result<(), String> {
    state::journal::append(conn, source, &Payload::Merge { merged_key: None }).map(|_| ())
}

fn set_name(conn: &Connection, thread_key: &str, name: Option<&str>) -> Result<(), String> {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => state::write::set_rename(conn, thread_key, name),
        None => state::write::clear_rename(conn, thread_key),
    }
}

/// Makes several threads show as one. The first key is the thread the rest join, which is the key
/// the banner's Unmerge undoes from and the key every list shows the merged thread under.
pub fn merge(
    conn: &Connection,
    keys: &[String],
    name: Option<&str>,
) -> Result<(String, Vec<String>, Option<String>), String> {
    let merged_key = keys.first().ok_or("a merge needs at least two threads")?.clone();
    let sources: Vec<String> = keys
        .iter()
        .skip(1)
        .filter(|key| **key != merged_key)
        .cloned()
        .collect();
    if sources.is_empty() {
        return Err("a merge needs at least two threads".into());
    }
    let rename_before = state::read::rename_of(conn, &merged_key)?;
    state::write::merge_threads(conn, &sources, &merged_key)?;

    let typed = name.map(str::trim).filter(|name| !name.is_empty());
    let chosen = match typed {
        Some(name) => Some(name.to_string()),
        None => longest_subject(conn, keys)?,
    };
    // A name that says exactly what the subject already says would make the pane print
    // "renamed, was ..." about nothing.
    if chosen.is_some() && chosen != subject_of(conn, &merged_key)? {
        set_name(conn, &merged_key, chosen.as_deref())?;
    }
    Ok((merged_key, sources, rename_before))
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

/// What each of these reverses is a row in the state database rather than a set of provider labels,
/// so they share a stack with each other and not with `mirror`.
///
/// Every variant but the last names one account, because a note, a name and a merge are all about a
/// single thread. Ignore and notify take a selection, and a selection can span two mailboxes.
pub enum What {
    Note {
        account_id: String,
        note: Note,
    },
    Rename {
        account_id: String,
        before: Vec<(String, Option<String>)>,
    },
    Merge {
        account_id: String,
        merged_key: String,
        sources: Vec<String>,
        rename_before: Option<String>,
    },
    Unmerge {
        account_id: String,
        merged_key: String,
        sources: Vec<String>,
    },
    Flags {
        before: Vec<(String, Vec<(String, ThreadFlags)>)>,
    },
}

pub struct UndoDecision {
    pub label: String,
    pub what: What,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that change can no longer be taken back")?;
    let db = db_of(app)?;
    match &entry.what {
        What::Note { account_id, note } => db.with(account_id, |conn| restore_note(conn, note))?,
        What::Rename { account_id, before } => db.with(account_id, |conn| {
            for (key, name) in before {
                set_name(conn, key, name.as_deref())?;
            }
            Ok(())
        })?,
        What::Merge {
            account_id,
            merged_key,
            sources,
            rename_before,
        } => db.with(account_id, |conn| {
            for source in sources {
                unmerge_one(conn, source)?;
            }
            set_name(conn, merged_key, rename_before.as_deref())
        })?,
        What::Unmerge {
            account_id,
            merged_key,
            sources,
        } => db.with(account_id, |conn| {
            state::write::merge_threads(conn, sources, merged_key)
        })?,
        What::Flags { before } => {
            for (account_id, held) in before {
                db.with(account_id, |conn| {
                    for (key, flags) in held {
                        state::write::set_thread_flags(
                            conn,
                            key,
                            Some(flags.ignored),
                            Some(flags.notify),
                        )?;
                    }
                    Ok(())
                })?;
            }
        }
    }
    crate::emit_store_changed(app, "threads state");
    Ok(())
}

fn handed_back(label: String, what: What) -> Undo {
    let token = UNDO.push(
        UndoDecision {
            label: label.clone(),
            what,
        },
        &label,
    );
    Undo {
        token,
        label,
        undo_ms: 0,
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn note_add(app: tauri::AppHandle, thread_key: String, body: String) -> Result<Note, String> {
    let db = db_of(&app)?;
    let account_id = holders(db.inner(), std::slice::from_ref(&thread_key))?
        .into_iter()
        .next()
        .map(|(account_id, _)| account_id)
        .ok_or("that thread is not on this device")?;
    let note = db.with(&account_id, |conn| add_note(conn, &thread_key, &body))?;
    crate::emit_store_changed(&app, &format!("threads thread:{thread_key} state"));
    Ok(note)
}

#[tauri::command(async)]
pub fn note_update(app: tauri::AppHandle, id: String, body: String) -> Result<(), String> {
    let db = db_of(&app)?;
    let account_id = account_holding(
        db.inner(),
        "SELECT COUNT(*) FROM state.notes WHERE id = ?1",
        &id,
    )
    .map_err(|_| "that note is not here".to_string())?;
    let thread_key = db.with(&account_id, |conn| {
        state::write::edit_note(conn, &id, &body)?;
        Ok(note_row(conn, &id)?.map(|(note, _)| note.thread_key))
    })?;
    let scope = thread_key
        .map(|key| format!("threads thread:{key} state"))
        .unwrap_or_else(|| "threads state".to_string());
    crate::emit_store_changed(&app, &scope);
    Ok(())
}

#[tauri::command(async)]
pub fn note_delete(app: tauri::AppHandle, id: String) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let account_id = account_holding(
        db.inner(),
        "SELECT COUNT(*) FROM state.notes WHERE id = ?1",
        &id,
    )
    .map_err(|_| "that note is not here".to_string())?;
    let note = db.with(&account_id, |conn| {
        let held = note_row(conn, &id)?
            .filter(|(_, deleted)| !deleted)
            .map(|(note, _)| note)
            .ok_or_else(|| "that note is not here".to_string())?;
        state::write::delete_note(conn, &id)?;
        Ok(held)
    })?;
    let scope = format!("threads thread:{} state", note.thread_key);
    crate::emit_store_changed(&app, &scope);
    Ok(handed_back(
        "Note deleted".to_string(),
        What::Note { account_id, note },
    ))
}

/// A null name is back to the real subject, which is what the pane's "renamed · was …" is offering.
#[tauri::command(async)]
pub fn thread_rename(
    app: tauri::AppHandle,
    key: String,
    name: Option<String>,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let account_id = holders(db.inner(), std::slice::from_ref(&key))?
        .into_iter()
        .next()
        .map(|(account_id, _)| account_id)
        .ok_or("that thread is not on this device")?;
    let before = db.with(&account_id, |conn| {
        let before = state::read::rename_of(conn, &key)?;
        set_name(conn, &key, name.as_deref())?;
        Ok(before)
    })?;
    let label = match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(_) => "Renamed".to_string(),
        None => "Name put back".to_string(),
    };
    crate::emit_store_changed(&app, &format!("threads thread:{key} state"));
    Ok(handed_back(
        label,
        What::Rename {
            account_id,
            before: vec![(key, before)],
        },
    ))
}

#[tauri::command(async)]
pub fn thread_merge(
    app: tauri::AppHandle,
    keys: Vec<String>,
    name: Option<String>,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    // One account, and it is the first key's: a merge across two mailboxes would be a thread whose
    // messages no single provider holds, and neither list could show it.
    let first = keys
        .first()
        .ok_or("a merge needs at least two threads")?
        .clone();
    let (account_id, held) = holders(db.inner(), &keys)?
        .into_iter()
        .find(|(_, held)| held.contains(&first))
        .ok_or("those threads are not on this device")?;
    let ordered: Vec<String> = keys.iter().filter(|key| held.contains(key)).cloned().collect();
    let (merged_key, sources, rename_before) =
        db.with(&account_id, |conn| merge(conn, &ordered, name.as_deref()))?;

    let label = format!("Merged {} threads", sources.len() + 1);
    crate::emit_store_changed(&app, &format!("threads thread:{merged_key} state"));
    Ok(handed_back(
        label,
        What::Merge {
            account_id,
            merged_key,
            sources,
            rename_before,
        },
    ))
}

#[tauri::command(async)]
pub fn thread_unmerge(app: tauri::AppHandle, key: String) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let account_id = holders(db.inner(), std::slice::from_ref(&key))?
        .into_iter()
        .next()
        .map(|(account_id, _)| account_id)
        .ok_or("that thread is not on this device")?;
    let sources = db.with(&account_id, |conn| {
        let sources = direct_sources(conn, &key)?;
        if sources.is_empty() {
            return Err("that thread is not a merge".to_string());
        }
        state::write::unmerge(conn, &key)?;
        Ok(sources)
    })?;
    crate::emit_store_changed(&app, "threads state");
    Ok(handed_back(
        "Unmerged".to_string(),
        What::Unmerge {
            account_id,
            merged_key: key,
            sources,
        },
    ))
}

/// An ignored thread still receives its messages and still appends them. What changes is that it
/// stops coming back to New, which `mirror::read` decides when it decides the group.
#[tauri::command(async)]
pub fn thread_ignore(app: tauri::AppHandle, keys: Vec<String>, on: bool) -> Result<Undo, String> {
    flags(&app, &keys, Some(on), None, |count| {
        plural(count, if on { "Ignoring" } else { "No longer ignoring" })
    })
}

#[tauri::command(async)]
pub fn thread_notify(app: tauri::AppHandle, keys: Vec<String>, on: bool) -> Result<Undo, String> {
    flags(&app, &keys, None, Some(on), |count| {
        plural(
            count,
            if on {
                "Notifications on"
            } else {
                "Notifications off"
            },
        )
    })
}

fn flags(
    app: &tauri::AppHandle,
    keys: &[String],
    ignored: Option<bool>,
    notify: Option<bool>,
    label: impl Fn(usize) -> String,
) -> Result<Undo, String> {
    let db = db_of(app)?;
    let mut before: Vec<(String, Vec<(String, ThreadFlags)>)> = Vec::new();
    let mut touched = 0usize;
    for (account_id, held) in holders(db.inner(), keys)? {
        let held = db.with(&account_id, |conn| {
            let mut before = Vec::new();
            for key in &held {
                before.push((key.clone(), state::read::flags_of(conn, key)?));
                state::write::set_thread_flags(conn, key, ignored, notify)?;
            }
            Ok(before)
        })?;
        touched += held.len();
        before.push((account_id, held));
    }
    let label = label(touched);
    crate::emit_store_changed(app, "threads state");
    Ok(handed_back(label, What::Flags { before }))
}

#[cfg(test)]
mod tests;
