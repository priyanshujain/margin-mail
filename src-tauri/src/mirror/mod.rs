// The mirror: one account's mailbox, as far back as its storage window reaches.
//
// `schema` is frozen contract. `read` and `write` are the only modules allowed to touch these
// tables, and `evict` is the only one allowed to remove anything from them.

pub mod evict;
pub mod fresh;
pub mod fts;
pub mod read;
pub mod schema;
pub mod write;

#[cfg(test)]
mod places_tests;
#[cfg(test)]
mod seen_tests;

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use tauri::Manager;

use crate::db::Db;
use crate::dto::{FlagPatch, LabelInfo, StorageUsed, ThreadPage, ThreadQuery, ThreadView, Undo};
use crate::sync::{self, outbox, Scoped};

/// Undo holds the state before a change rather than a description of it, because a bulk archive of
/// forty threads with mixed prior state cannot be reversed from what the frontend knew, and a
/// reversal that guesses is worse than no reversal at all.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub token: String,
    pub label: String,
    /// The prior labels of every message the change touched, per account.
    pub prior: Vec<(String, Vec<write::PriorFlags>)>,
}

/// Deep enough that nobody reaches the bottom of it in a session, shallow enough that it is not a
/// second copy of the mailbox.
const UNDO_DEPTH: usize = 50;

fn undo_stack() -> &'static Mutex<VecDeque<UndoEntry>> {
    static STACK: OnceLock<Mutex<VecDeque<UndoEntry>>> = OnceLock::new();
    STACK.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn undo_push(entry: UndoEntry) -> String {
    let token = entry.token.clone();
    crate::undo::note(&token, &entry.label);
    if let Ok(mut stack) = undo_stack().lock() {
        stack.push_front(entry);
        while stack.len() > UNDO_DEPTH {
            stack.pop_back();
        }
    }
    token
}

/// The token at the top of the stack, which is what `z` reverses. None when there is nothing to
/// take back, and pressing `z` then does nothing at all rather than saying so.
pub fn undo_top() -> Option<String> {
    let stack = undo_stack().lock().ok()?;
    stack.front().map(|entry| entry.token.clone())
}

/// What the top of the stack is called, for the toast that offers it.
pub fn undo_top_label() -> Option<String> {
    let stack = undo_stack().lock().ok()?;
    stack.front().map(|entry| entry.label.clone())
}

pub fn undo_take(token: &str) -> Option<UndoEntry> {
    crate::undo::forget(token);
    let mut stack = undo_stack().lock().ok()?;
    let at = stack.iter().position(|entry| entry.token == token)?;
    stack.remove(at)
}

/// What the undo package calls. It puts every message back exactly as it was and queues the
/// difference to the provider, which is why the prior state is held rather than the action.
pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    // Start fresh is the one reversal measured in days rather than seconds, so its record is on
    // disk and outlives this stack. Falling back here rather than giving the frontend a second
    // command to know about is what keeps `z` and every toast one call deep.
    let Some(entry) = undo_take(token) else {
        return fresh::undo(app, token).map(|_| ());
    };
    let db = app
        .try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())?;
    for (account_id, prior) in &entry.prior {
        db.with(account_id, |conn| {
            for message in prior {
                let current = write::prior_flags(conn, std::slice::from_ref(&message.id))?;
                let now: Vec<String> = current
                    .first()
                    .map(|held| held.labels.clone())
                    .unwrap_or_default();
                let add: Vec<String> = message
                    .labels
                    .iter()
                    .filter(|label| !now.contains(label))
                    .cloned()
                    .collect();
                let remove: Vec<String> = now
                    .iter()
                    .filter(|label| !message.labels.contains(label))
                    .cloned()
                    .collect();
                if add.is_empty() && remove.is_empty() {
                    continue;
                }
                // `apply_labels` rather than `set_labels`. The latter replays whatever is still
                // waiting in the outbox over what it is handed, which is right for a word from the
                // provider and wrong for a reversal, whose whole point is to overrule what is
                // waiting.
                write::apply_labels(conn, std::slice::from_ref(&message.id), &add, &remove)?;
                outbox::queue_labels(conn, std::slice::from_ref(&message.id), &add, &remove, None)?;
            }
            Ok(())
        })?;
    }
    crate::emit_store_changed(app, "threads");
    Ok(())
}

fn db_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, Db>, String> {
    app.try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        one.to_string()
    } else {
        format!("{one} {count} threads")
    }
}

/// What the toast says. The patch is what the user chose, so the sentence comes from the patch
/// rather than from the caller naming it twice.
fn flag_label(patch: &FlagPatch, count: usize) -> String {
    let verb = match (
        patch.archived,
        patch.trashed,
        patch.spam,
        patch.starred,
        patch.seen,
    ) {
        (Some(true), _, _, _, _) => "Archived",
        (Some(false), _, _, _, _) => "Moved to the Inbox",
        (_, Some(true), _, _, _) => "Trashed",
        (_, Some(false), _, _, _) => "Restored",
        (_, _, Some(true), _, _) => "Reported as spam",
        (_, _, Some(false), _, _) => "Marked as not spam",
        (_, _, _, Some(true), _) => "Starred",
        (_, _, _, Some(false), _) => "Unstarred",
        (_, _, _, _, Some(true)) => "Marked as seen",
        (_, _, _, _, Some(false)) => "Marked as unseen",
        _ => "Changed",
    };
    plural(count, verb)
}

/// Lands a flag change in every account that holds one of these threads, queues the push, and
/// keeps what the flags were so it can be taken back.
fn apply_and_queue(
    db: &Db,
    keys: &[String],
    patch: &FlagPatch,
    label: impl Fn(usize) -> String,
) -> Result<Undo, String> {
    let mut prior_by_account = Vec::new();
    let mut touched: Vec<String> = Vec::new();
    for (account_id, _) in sync::accounts(db, None) {
        let (prior, keys_here) = db.with(&account_id, |conn| {
            let ids = write::message_ids_for_keys(conn, keys)?;
            if ids.is_empty() {
                return Ok((Vec::new(), Vec::new()));
            }
            let prior = write::prior_flags(conn, &ids)?;
            write::apply_flags(conn, &ids, patch)?;
            outbox::queue_flags(conn, &ids, patch, keys.first().map(String::as_str))?;
            Ok((prior, held_keys(conn, keys)?))
        })?;
        if !prior.is_empty() {
            prior_by_account.push((account_id, prior));
            touched.extend(keys_here);
        }
    }
    touched.sort();
    touched.dedup();

    let entry = UndoEntry {
        token: write::fresh_id("undo"),
        label: label(touched.len()),
        prior: prior_by_account,
    };
    let shown = entry.label.clone();
    Ok(Undo {
        token: undo_push(entry),
        label: shown,
        undo_ms: 0,
    })
}

/// Which of these keys this account actually holds, so a toast over several accounts counts each
/// thread once rather than once per account that looked.
fn held_keys(conn: &rusqlite::Connection, keys: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for key in keys {
        let held: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE thread_key = ?1",
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

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn threads_list(app: tauri::AppHandle, query: ThreadQuery) -> Result<ThreadPage, String> {
    let db = db_of(&app)?;
    sync::list_pages(db.inner(), &query)
}

/// Which account holds a thread. Every command that takes a thread key starts here, because the
/// mirror is one database per account and a key does not say which one it came from.
///
/// One index seek per account, stopping at the first that says yes.
fn account_holding(db: &Db, key: &str) -> Result<String, String> {
    for (account_id, _) in sync::accounts(db, None) {
        if db.with(&account_id, |conn| read::holds_thread(conn, key))? {
            return Ok(account_id);
        }
    }
    Err("that thread is not on this device".to_string())
}

/// Opening a thread is a read of the mirror and nothing else: it fetches nothing and waits for
/// nothing. Everything that makes a thread look like a thread is already on the device, so a
/// message whose body has not been cached yet comes back with `body_pending` set rather than being
/// left out, and `thread_hydrate` fills it in afterwards.
///
/// This used to await the missing bodies first, one Gmail round trip after another, before a
/// single row was read. A five message thread that had not been cached cost five serial fetches
/// before anything could be drawn, and that is the several seconds an open used to take.
///
/// A render made by an older sanitiser is made again on the way past, which is local and cheap.
/// Marking the thread seen is `thread_opened`, which the frontend calls once the view is on
/// screen, so that a read of the mirror stays a read and Focus & Reply can mark what it shows
/// without going through the pane.
#[tauri::command(async)]
pub fn thread_view(app: tauri::AppHandle, key: String) -> Result<ThreadView, String> {
    let db = db_of(&app)?;
    let account_id = account_holding(db.inner(), &key)?;

    db.with(&account_id, |conn| {
        // A snoozed thread that came back waits in the Back group until it is opened, and this is
        // the moment it is opened. Nothing else in the app knows when that happens.
        crate::snooze::opened(conn, &key)?;

        let options = sync::hydrate::render_options(conn)?;
        for message_id in read::stale_renders(conn, &key)? {
            write::rerender(conn, &message_id, &options)?;
        }
        let own = options.own_addresses.clone();
        read::thread_view(conn, &account_id, &key, &own)
    })
}

/// The other half of an open: the bodies the mirror did not have, asked for together rather than
/// one after another, and stored as they land. Returns how many arrived.
///
/// It emits `store-changed` scoped to `thread` when any did, which is what tells the pane already
/// showing the headers to ask for the thread again and fill itself in.
///
/// A body that will not come does not fail this. A thread where four of five arrived is a thread
/// worth reading, and the fifth is still missing, so whatever asks next asks again.
#[tauri::command]
pub async fn thread_hydrate(app: tauri::AppHandle, key: String) -> Result<u32, String> {
    let db = db_of(&app)?;
    let account_id = account_holding(db.inner(), &key)?;
    let Some(remote) = sync::remote_for(&account_id) else {
        return Ok(0);
    };
    let store = Scoped {
        db: db.inner(),
        account_id: &account_id,
    };
    let fetched = sync::hydrate::thread_bodies(&store, remote.as_ref(), &key)
        .await
        .map_err(|e| e.to_string())?;
    if fetched > 0 {
        crate::emit_store_changed(&app, "thread");
    }
    Ok(fetched)
}

/// Opened means seen. The one place that knows it, so the pane, the one-column page and Focus &
/// Reply cannot each decide differently.
///
/// Not a triage action and not on the undo stack: `z` after opening a thread has to take back the
/// archive before it, not the reading of this one. Ignore is about New for you and notifications
/// rather than read state, so an ignored thread is marked like any other. Every unseen message of
/// the thread goes, because `threads.unseen` is any of them, and a thread with nothing unseen
/// writes nothing, queues nothing and answers false.
pub fn seen_on_open(conn: &rusqlite::Connection, key: &str) -> Result<bool, String> {
    let ids = read::unseen_message_ids(conn, key)?;
    if ids.is_empty() {
        return Ok(false);
    }
    let patch = FlagPatch {
        seen: Some(true),
        ..FlagPatch::default()
    };
    write::apply_flags(conn, &ids, &patch)?;
    outbox::queue_flags(conn, &ids, &patch, Some(key))?;
    Ok(true)
}

/// The mirror's half of opening a thread, called by whoever has just put it on screen.
///
/// It deliberately emits no `store-changed`. The badge counts New for you and this has just taken
/// one off it, but the only scope the badge listens for is `threads`, and `threads` reloads the
/// list: the row you are reading would drop from New for you into the seen band under your eyes.
/// The frontend has already taken the dot off the row and the row moves on the next reload the
/// sync loop causes anyway, so the badge is asked directly, the way launch asks it.
#[tauri::command(async)]
pub fn thread_opened(app: tauri::AppHandle, key: String) -> Result<(), String> {
    let db = db_of(&app)?;
    let account_id = account_holding(db.inner(), &key)?;
    if db.with(&account_id, |conn| seen_on_open(conn, &key))? {
        crate::badge::refresh(&app);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn flags_set(
    app: tauri::AppHandle,
    keys: Vec<String>,
    patch: FlagPatch,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let undo = apply_and_queue(db.inner(), &keys, &patch, |count| flag_label(&patch, count))?;
    crate::emit_store_changed(&app, "threads");
    Ok(undo)
}

/// Everything in a place, seen. The place is served by the same view the list is, so "everything"
/// means exactly what the person was looking at.
#[tauri::command(async)]
pub fn mark_all_seen(
    app: tauri::AppHandle,
    account_id: Option<String>,
    place: String,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let place = read::place_named(&place).ok_or("there is no such place")?;
    let mut keys = Vec::new();
    let mut cursor = None;
    loop {
        let page = sync::list_pages(
            db.inner(),
            &ThreadQuery {
                account_id: account_id.clone(),
                place,
                label_id: None,
                query: None,
                limit: 200,
                cursor,
            },
        )?;
        keys.extend(page.threads.into_iter().filter(|t| t.unseen).map(|t| t.key));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let patch = FlagPatch {
        seen: Some(true),
        ..FlagPatch::default()
    };
    let undo = apply_and_queue(db.inner(), &keys, &patch, |count| {
        plural(count, "Marked as seen")
    })?;
    crate::emit_store_changed(&app, "threads");
    Ok(undo)
}

#[tauri::command(async)]
pub fn labels_list(
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<Vec<LabelInfo>, String> {
    let db = db_of(&app)?;
    let mut out = Vec::new();
    for (id, _) in sync::accounts(db.inner(), account_id.as_deref()) {
        out.extend(db.with(&id, |conn| read::labels(conn, &id))?);
    }
    Ok(out)
}

fn apply_labels_and_queue(
    db: &Db,
    keys: &[String],
    add: &[String],
    remove: &[String],
    label: impl Fn(usize) -> String,
) -> Result<Undo, String> {
    let mut prior_by_account = Vec::new();
    let mut touched: Vec<String> = Vec::new();
    for (account_id, _) in sync::accounts(db, None) {
        let (prior, keys_here) = db.with(&account_id, |conn| {
            let ids = write::message_ids_for_keys(conn, keys)?;
            if ids.is_empty() {
                return Ok((Vec::new(), Vec::new()));
            }
            let prior = write::prior_flags(conn, &ids)?;
            write::apply_labels(conn, &ids, add, remove)?;
            outbox::queue_labels(conn, &ids, add, remove, keys.first().map(String::as_str))?;
            Ok((prior, held_keys(conn, keys)?))
        })?;
        if !prior.is_empty() {
            prior_by_account.push((account_id, prior));
            touched.extend(keys_here);
        }
    }
    touched.sort();
    touched.dedup();
    let entry = UndoEntry {
        token: write::fresh_id("undo"),
        label: label(touched.len()),
        prior: prior_by_account,
    };
    let shown = entry.label.clone();
    Ok(Undo {
        token: undo_push(entry),
        label: shown,
        undo_ms: 0,
    })
}

#[tauri::command(async)]
pub fn label_apply(
    app: tauri::AppHandle,
    keys: Vec<String>,
    label_id: String,
    on: bool,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let (add, remove) = if on {
        (vec![label_id], Vec::new())
    } else {
        (Vec::new(), vec![label_id])
    };
    let undo = apply_labels_and_queue(db.inner(), &keys, &add, &remove, |count| {
        plural(count, if on { "Labelled" } else { "Unlabelled" })
    })?;
    crate::emit_store_changed(&app, "threads");
    Ok(undo)
}

/// A move is a label applied and the Inbox removed, which is what "move" means to a provider whose
/// folders are labels.
#[tauri::command(async)]
pub fn label_move(
    app: tauri::AppHandle,
    keys: Vec<String>,
    label_id: String,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let undo = apply_labels_and_queue(
        db.inner(),
        &keys,
        &[label_id],
        &[write::LABEL_INBOX.to_string()],
        |count| plural(count, "Moved"),
    )?;
    crate::emit_store_changed(&app, "threads");
    Ok(undo)
}

/// Throws the mirror away and lets the next pass fill it again. The state database is untouched.
#[tauri::command(async)]
pub fn mirror_clear(app: tauri::AppHandle, account_id: String) -> Result<(), String> {
    let db = db_of(&app)?;
    db.with(&account_id, write::clear)?;
    crate::emit_store_changed(&app, "threads");
    Ok(())
}

#[tauri::command(async)]
pub fn storage_used(app: tauri::AppHandle) -> Result<Vec<StorageUsed>, String> {
    let db = db_of(&app)?;
    let mut out = Vec::new();
    for (id, _) in sync::accounts(db.inner(), None) {
        out.push(db.with(&id, |conn| read::storage_used(conn, &id))?);
    }
    Ok(out)
}
