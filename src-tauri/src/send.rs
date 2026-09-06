// The send pipeline: the undo delay, the outbox row, and the one operation in this app that is not
// safe to do twice.
//
// Every send is held. The row goes into the outbox with a `hold_until` a few seconds out, the toast
// counts that down, and cancelling inside it deletes the row and puts the draft back. Nothing has
// left the machine until the hold expires, which is what makes the Undo honest rather than a race
// against a request already in the air.
//
// After that it is the outbox's ordinary story with one difference. A flag change pushed twice is
// the same flag change; a message sent twice is a message somebody has to apologise for. So the
// bytes are frozen at queue time with a `Message-ID` stamped into them, every attempt is those same
// bytes, and a row that has been attempted before asks the mirror whether the message it is holding
// is already in the mailbox before it tries again. That is the answer to the process dying between
// the provider accepting the message and the row being deleted: the row survives, the next drain
// finds the sent message under its own `Message-ID` once a sync has brought it back, and the row is
// dropped rather than sent again.
//
// Two drains can be in the loop at once, because the undo delay kicks one of its own so the message
// leaves on time rather than at the next poll. The lease is a compare and set on the row's hold, so
// exactly one of them gets it.

use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::decisions::db_of;
use crate::drafts;
use crate::dto::{Draft, Outgoing, Person, SnoozeKind, Undo};
use crate::mirror::write::{self, OutboxRow};
use crate::provider::ProviderError;
use crate::snooze;
use crate::sync::{self, outbox, Remote, Store};
use crate::undo::Stack;

static UNDO: Stack<UndoSend> = Stack::new("send");

/// How long a drain holds a send while it is making the call. Long enough that a slow upload is not
/// picked up by the next pass, short enough that a process that died mid send is retried in a
/// minute rather than an hour.
const LEASE_MS: i64 = 60_000;

/// What the lease says when another drain already has the row. Not a failure: nothing is wrong, and
/// the drain holding it is the one that will say what happened.
pub const TAKEN: &str = "that send is already in flight";

/// The most `References` entries to carry. The first one is the thread and the rest are context, so
/// a long thread keeps its head and its tail rather than growing a header without a limit.
const MAX_REFERENCES: usize = 20;

// ---------------------------------------------------------------------------------------------
// The row
// ---------------------------------------------------------------------------------------------

/// An outbox row's payload for a send.
///
/// The draft is flattened into it so a row carrying nothing but a serialised `dto::Draft`, which is
/// what `unsubscribe` queues, reads back as one of these with the rest of the fields empty. Such a
/// row is completed on its first pass and frozen from then on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendPayload {
    #[serde(flatten)]
    pub draft: Draft,
    /// The built message, base64. Frozen at queue time, so a retry is the same bytes with the same
    /// `Message-ID` and a file moved after the send was queued cannot fail it.
    #[serde(default)]
    pub raw: Option<String>,
    /// The `Message-ID` stamped into those bytes, which is what makes an interrupted send
    /// answerable.
    #[serde(default)]
    pub message_id: Option<String>,
    /// The provider's own thread id, so a reply lands in the thread on their side too.
    #[serde(default)]
    pub thread_hint: Option<String>,
}

impl SendPayload {
    fn new(draft: Draft) -> SendPayload {
        SendPayload {
            draft,
            raw: None,
            message_id: None,
            thread_hint: None,
        }
    }
}

/// The facts a reply needs from the thread it is replying to. `mime::build` writes the headers;
/// this is where they come from.
#[derive(Debug, Clone, Default)]
pub struct Threading {
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub thread_hint: Option<String>,
    pub subject: String,
}

/// One row of the thread being replied to, in the order it arrived.
struct Held {
    id: String,
    message_id: Option<String>,
    provider_thread_id: String,
    subject: String,
}

/// What a reply has to carry to land in the thread on both ends.
///
/// `Draft::in_reply_to` names the message being answered. The contract does not say whether that is
/// the provider's id or the RFC header, and both reach the composer, so both are looked for and the
/// newest message in the thread is what a draft that names neither replies to.
pub fn threading(conn: &Connection, draft: &Draft) -> Result<Threading, String> {
    let mut facts = Threading {
        subject: draft.subject.clone(),
        ..Threading::default()
    };
    let Some(key) = draft.thread_key.as_deref().filter(|key| !key.is_empty()) else {
        return Ok(facts);
    };

    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.message_id, m.provider_thread_id, m.subject
               FROM messages m
               JOIN threads t ON t.provider_thread_id = m.provider_thread_id
              WHERE t.thread_key = ?1
              ORDER BY m.date_ms ASC, m.id ASC",
        )
        .map_err(|e| e.to_string())?;
    let held: Vec<Held> = stmt
        .query_map([key], |row| {
            Ok(Held {
                id: row.get(0)?,
                message_id: row.get(1)?,
                provider_thread_id: row.get(2)?,
                subject: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if held.is_empty() {
        return Ok(facts);
    }

    let named = draft.in_reply_to.as_deref().unwrap_or_default();
    let at = held
        .iter()
        .position(|row| {
            row.id == named || row.message_id.as_deref() == Some(named).filter(|n| !n.is_empty())
        })
        .unwrap_or(held.len() - 1);
    let parent = &held[at];

    facts.in_reply_to = parent.message_id.clone().map(angled);
    facts.thread_hint = Some(parent.provider_thread_id.clone());

    let mut references: Vec<String> = held[..=at]
        .iter()
        .filter_map(|row| row.message_id.clone())
        .map(angled)
        .collect();
    references.dedup();
    if references.len() > MAX_REFERENCES {
        let head = references[0].clone();
        references = references.split_off(references.len() - (MAX_REFERENCES - 1));
        references.insert(0, head);
    }
    facts.references = references;

    if facts.subject.trim().is_empty() {
        facts.subject = replying_to(&parent.subject);
    }
    Ok(facts)
}

fn angled(message_id: String) -> String {
    if message_id.starts_with('<') {
        message_id
    } else {
        format!("<{message_id}>")
    }
}

/// A subject that matches the thread, which is half of how a reply is threaded by clients that do
/// not read `References`. Nothing is prefixed twice.
fn replying_to(subject: &str) -> String {
    let subject = subject.trim();
    if subject.len() >= 3 && subject[..3].eq_ignore_ascii_case("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

/// The `Message-ID` this message will carry, on the sender's own domain, which is what a receiving
/// server expects and what makes an interrupted send findable afterwards.
fn stamp(from: &str) -> String {
    let domain = from.rsplit_once('@').map(|(_, domain)| domain).unwrap_or("margin.mail");
    format!("<{}@{}>", write::fresh_id("margin"), domain)
}

// ---------------------------------------------------------------------------------------------
// Who it is from
// ---------------------------------------------------------------------------------------------

/// The address this account sends as, for a caller that has no app handle. The mirror's `meta`
/// first, then the newest message the account has sent, because an outbox drain has no account
/// registry to ask and a message with no From cannot be built at all.
pub fn own_address(conn: &Connection) -> Result<Option<String>, String> {
    if let Some(address) =
        write::meta_get(conn, write::OWN_ADDRESS_KEY)?.filter(|address| !address.is_empty())
    {
        return Ok(Some(address));
    }
    conn.query_row(
        "SELECT from_address FROM messages WHERE sent = 1 AND from_address <> ''
         ORDER BY date_ms DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// The address a particular draft goes out as: a verified alias when the composer chose one, and the
/// account's own otherwise.
pub fn sender(draft: &Draft, from: &Person) -> Person {
    match draft.from_alias.as_deref().filter(|alias| !alias.trim().is_empty()) {
        Some(alias) => Person {
            name: from.name.clone(),
            address: alias.to_string(),
        },
        None => from.clone(),
    }
}

/// The same, with the name on it, for a caller that has one. The address is written into the
/// mirror's `meta` on the way past so the drain can find it later without the registry.
pub fn own_person(app: &tauri::AppHandle, db: &Db, account_id: &str) -> Result<Person, String> {
    let entry = crate::accounts::find(app, account_id)?;
    let address = match entry.as_ref().map(|entry| entry.email.clone()) {
        Some(email) if !email.is_empty() => email,
        _ => db
            .with(account_id, own_address)?
            .ok_or("this account has no address to send from yet")?,
    };
    db.with(account_id, |conn| {
        write::meta_set(conn, write::OWN_ADDRESS_KEY, &address)
    })?;
    Ok(Person {
        name: entry
            .map(|entry| entry.name)
            .filter(|name| !name.trim().is_empty()),
        address,
    })
}

// ---------------------------------------------------------------------------------------------
// The drain
// ---------------------------------------------------------------------------------------------

/// True when this error is another drain holding the row rather than something going wrong.
pub fn taken(error: &ProviderError) -> bool {
    matches!(error, ProviderError::Other(message) if message == TAKEN)
}

/// Whether the message this row holds is already in the mailbox. Answerable only because the bytes
/// carry a `Message-ID` this app stamped and the mirror keeps that header on every row it has.
fn already_sent(conn: &Connection, message_id: &str) -> Result<bool, String> {
    let bare = write::bare_id(message_id).unwrap_or_else(|| message_id.to_string());
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE message_id = ?1",
            [&bare],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

/// Takes the row, if it is still there to be taken. The compare is on the hold the caller read, so
/// two drains racing for one send end with one of them holding it and the other told so.
fn lease(conn: &Connection, id: &str, expected: i64, until: i64) -> Result<bool, String> {
    conn.execute(
        "UPDATE outbox SET attempts = attempts + 1, hold_until = ?3
          WHERE id = ?1 AND hold_until = ?2",
        rusqlite::params![id, expected, until],
    )
    .map(|changed| changed > 0)
    .map_err(|e| e.to_string())
}

/// A failed send waits and says why, without counting a second attempt: the lease already counted
/// this one.
pub fn deferred(conn: &Connection, id: &str, until: i64, error: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE outbox SET hold_until = ?2, last_error = ?3 WHERE id = ?1",
        rusqlite::params![id, until, error],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn other<E: std::fmt::Display>(error: E) -> ProviderError {
    ProviderError::Other(error.to_string())
}

/// Builds the message a row was queued with nothing but a draft for, and writes it back onto the
/// row. From here on every attempt is these bytes.
fn complete<S: Store>(store: &S, row: &OutboxRow, payload: &mut SendPayload) -> Result<(), String> {
    let draft = payload.draft.clone();
    let (raw, message_id, thread_hint) = store.with(|conn| {
        let address =
            own_address(conn)?.ok_or("this account has no address to send from yet")?;
        let from = sender(&draft, &Person { name: None, address });
        let facts = threading(conn, &draft)?;
        let message_id = stamp(&from.address);
        let raw = drafts::built(
            conn,
            &draft,
            &from,
            Some(&message_id),
            facts.in_reply_to.as_deref(),
            &facts.references,
            &facts.subject,
        )?;
        Ok((raw, message_id, facts.thread_hint))
    })?;

    payload.raw = Some(base64::engine::general_purpose::STANDARD.encode(&raw));
    payload.message_id = Some(message_id);
    payload.thread_hint = thread_hint;

    let written = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    store.with(|conn| {
        write::enqueue(
            conn,
            &OutboxRow {
                payload: written,
                ..row.clone()
            },
        )
        .map(|_| ())
    })
}

/// One send, from the outbox drain. `Ok` means the row can go: either the message went, or it had
/// already gone before the process died and the mirror can prove it.
pub async fn push<S: Store>(
    store: &S,
    remote: &dyn Remote,
    row: &OutboxRow,
) -> Result<(), ProviderError> {
    let mut payload: SendPayload = serde_json::from_str(&row.payload)
        .map_err(|e| ProviderError::Other(format!("that send cannot be read: {e}")))?;

    // Attempted before, so it may have been accepted before the process died. The mirror is asked
    // under the stamped `Message-ID`, and a message that is already there is a row to drop rather
    // than a message to send twice.
    if row.attempts > 0 {
        if let Some(message_id) = payload.message_id.clone() {
            if store
                .with(|conn| already_sent(conn, &message_id))
                .map_err(other)?
            {
                return Ok(());
            }
        }
    }

    if payload.raw.is_none() {
        complete(store, row, &mut payload).map_err(other)?;
    }
    let encoded = payload
        .raw
        .clone()
        .ok_or_else(|| ProviderError::Other("that send has no message on it".to_string()))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(other)?;

    if !store
        .with(|conn| lease(conn, &row.id, row.hold_until, write::now_ms() + LEASE_MS))
        .map_err(other)?
    {
        return Err(ProviderError::Other(TAKEN.to_string()));
    }

    remote.send(&raw, payload.thread_hint.as_deref()).await?;

    // The message has gone. A reminder that will not write is a reminder somebody does not get, and
    // it is not a reason to hand the row back to the queue and send the message a second time.
    if let Some(at) = payload.draft.remind_at_ms {
        let key = payload
            .draft
            .thread_key
            .clone()
            .or_else(|| payload.message_id.as_deref().and_then(write::bare_id));
        if let Some(key) = key {
            let _ = store.with(|conn| {
                snooze::set(conn, &[key], SnoozeKind::IfNoReply, at).map(|_| ())
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Queueing one
// ---------------------------------------------------------------------------------------------

/// The row a send becomes, held until `hold_until`.
pub fn queue(
    conn: &Connection,
    draft: &Draft,
    from: &Person,
    hold_until: i64,
) -> Result<String, String> {
    let from = sender(draft, from);
    let facts = threading(conn, draft)?;
    let message_id = stamp(&from.address);
    let raw = drafts::built(
        conn,
        draft,
        &from,
        Some(&message_id),
        facts.in_reply_to.as_deref(),
        &facts.references,
        &facts.subject,
    )?;
    if raw.len() as u64 > drafts::MAX_ENCODED_BYTES {
        return Err(format!(
            "That message is {} MB encoded, over the {} MB one message may be.",
            raw.len() as u64 / (1024 * 1024),
            drafts::MAX_ENCODED_BYTES / (1024 * 1024)
        ));
    }

    let payload = SendPayload {
        raw: Some(base64::engine::general_purpose::STANDARD.encode(&raw)),
        message_id: Some(message_id),
        thread_hint: facts.thread_hint,
        ..SendPayload::new(draft.clone())
    };
    write::enqueue(
        conn,
        &OutboxRow {
            op: write::OP_SEND.to_string(),
            payload: serde_json::to_string(&payload).map_err(|e| e.to_string())?,
            thread_key: draft.thread_key.clone(),
            hold_until,
            created_at: write::now_ms(),
            ..OutboxRow::default()
        },
    )
}

fn outgoing(row: &OutboxRow) -> Option<Outgoing> {
    let payload: SendPayload = serde_json::from_str(&row.payload).ok()?;
    Some(Outgoing {
        id: row.id.clone(),
        account_id: payload.draft.account_id,
        thread_key: row.thread_key.clone(),
        to: payload.draft.to,
        subject: payload.draft.subject,
        hold_until_ms: row.hold_until,
        attempts: row.attempts.max(0) as u32,
        last_error: row.last_error.clone(),
    })
}

fn sends(conn: &Connection) -> Result<Vec<Outgoing>, String> {
    Ok(write::outbox_rows(conn, 200)?
        .iter()
        .filter(|row| row.op == write::OP_SEND)
        .filter_map(outgoing)
        .collect())
}

/// Who the toast names.
fn named(to: &[Person]) -> String {
    let first = to
        .first()
        .map(|person| {
            person
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| person.address.clone())
        })
        .unwrap_or_else(|| "nobody".to_string());
    match to.len() {
        0 | 1 => first,
        2 => format!("{first} and 1 other"),
        many => format!("{first} and {} others", many - 1),
    }
}

/// Drains one account now rather than at the next poll, which is what the end of an undo delay and
/// Send now both want.
async fn drain(app: &tauri::AppHandle, account_id: &str) {
    let Ok(db) = db_of(app) else { return };
    let Some(remote) = sync::remote_for(account_id) else {
        return;
    };
    let store = sync::Scoped {
        db: db.inner(),
        account_id,
    };
    let deadline = write::now_ms() + outbox::PUSH_BUDGET_MS;
    let outcome = outbox::drain(&store, remote.as_ref(), deadline).await;
    if outcome.changed {
        crate::emit_store_changed(app, "threads outbox");
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn send(app: tauri::AppHandle, draft: Draft) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let account_id = draft.account_id.clone();
    let from = own_person(&app, db.inner(), &account_id)?;
    let delay_ms = crate::settings::load(&app)
        .map(|settings| settings.undo_delay_secs)
        .unwrap_or_else(|_| crate::settings::defaults().undo_delay_secs) as i64
        * 1_000;
    let hold_until = write::now_ms() + delay_ms;

    let id = db.with(&account_id, |conn| queue(conn, &draft, &from, hold_until))?;
    // The draft has become the message. Cancelling puts it back from the row's own payload, which is
    // why nothing here has to keep a second copy of it.
    let provider_draft_id = match draft.id.as_deref() {
        Some(draft_id) if !draft_id.is_empty() => {
            db.with(&account_id, |conn| drafts::delete(conn, draft_id))?
        }
        _ => None,
    };

    if let Some(provider_draft_id) = provider_draft_id {
        let handle = app.clone();
        let account_id = account_id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle;
            if let Some(provider) = crate::sync::remote_for(&account_id) {
                let _ = provider.draft_delete(&provider_draft_id).await;
            }
        });
    }

    // The hold is the whole promise, so the drain happens when it expires rather than whenever the
    // poll loop next comes round.
    let handle = app.clone();
    let waiting = account_id.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms.max(0) as u64)).await;
        drain(&handle, &waiting).await;
    });

    let label = format!("Sent to {}", named(&draft.to));
    let token = UNDO.push(
        UndoSend {
            account_id,
            outgoing_id: id,
            draft,
        },
        &label,
    );
    crate::emit_store_changed(&app, "threads outbox");
    Ok(Undo {
        token,
        label,
        undo_ms: delay_ms.max(0) as u32,
    })
}

/// Skips the rest of the hold. The row is already the message, so this is a moment brought forward
/// and not a second send.
#[tauri::command]
pub async fn send_now(app: tauri::AppHandle, outgoing_id: String) -> Result<(), String> {
    let db = db_of(&app)?;
    // The toast has the undo token rather than the outbox id, because that is what `send` handed
    // back, so either is accepted here. Otherwise the toast's Send now would have to guess which
    // row it belonged to, and the wrong guess sends the wrong message early.
    let outgoing_id = UNDO
        .peek(&outgoing_id)
        .map(|held| held.outgoing_id.clone())
        .unwrap_or(outgoing_id);

    for (account_id, _) in sync::accounts(db.inner(), None) {
        if !db.with(&account_id, |conn| release(conn, &outgoing_id))? {
            continue;
        }
        drain(&app, &account_id).await;
        crate::emit_store_changed(&app, "threads outbox");
        return Ok(());
    }
    Err("that message is not waiting to be sent".to_string())
}

#[tauri::command(async)]
pub fn outbox_list(app: tauri::AppHandle) -> Result<Vec<Outgoing>, String> {
    let db = db_of(&app)?;
    let mut all = Vec::new();
    for (account_id, _) in sync::accounts(db.inner(), None) {
        all.extend(db.with(&account_id, sends)?);
    }
    all.sort_by(|a, b| a.hold_until_ms.cmp(&b.hold_until_ms).then(a.id.cmp(&b.id)));
    Ok(all)
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

/// A send reverses by taking the row out of the outbox before its hold expires and putting the
/// draft back where it was. After the hold there is nothing to reverse, which is the point of
/// having one.
#[derive(Clone)]
pub struct UndoSend {
    pub account_id: String,
    pub outgoing_id: String,
    pub draft: Draft,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that message can no longer be taken back")?;
    let db = db_of(app)?;
    db.with(&entry.account_id, |conn| {
        cancel(conn, &entry.outgoing_id, &entry.draft)
    })?;
    crate::emit_store_changed(app, "threads outbox");
    Ok(())
}

/// Takes the row back out of the outbox and puts the draft back where it was. A row that has been
/// leased is a row somebody is already sending, and there is no taking that back.
pub fn cancel(conn: &Connection, outgoing_id: &str, draft: &Draft) -> Result<(), String> {
    let attempts: Option<i64> = conn
        .query_row(
            "SELECT attempts FROM outbox WHERE id = ?1",
            [outgoing_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match attempts {
        None => return Err("that message has already gone".to_string()),
        Some(attempts) if attempts > 0 => {
            return Err("that message is already on its way".to_string())
        }
        Some(_) => {}
    }
    write::dequeue(conn, outgoing_id)?;
    drafts::save(conn, draft).map(|_| ())
}

/// Brings the moment forward. The row is already the message, so Send now is a hold given up rather
/// than a second send.
pub fn release(conn: &Connection, outgoing_id: &str) -> Result<bool, String> {
    conn.execute(
        "UPDATE outbox SET hold_until = ?2 WHERE id = ?1 AND op = ?3",
        rusqlite::params![outgoing_id, write::now_ms(), write::OP_SEND],
    )
    .map(|changed| changed > 0)
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
