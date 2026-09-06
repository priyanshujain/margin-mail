// Drafts: the copy on this machine and the copy that roams.
//
// Two rhythms, and the reason there are two is the whole of this module. The local write is
// synchronous, costs a row, and happens on every save the composer sends; the provider upload is
// rate limited and coalesces, because a draft saved on a keystroke would be a request on a
// keystroke and Gmail's per-user quota is not a rounding error at that rate. What makes the
// coalescing free is that the `drafts` row is itself the queue: the payload carries the version the
// composer last wrote and the version the provider last had, and ten keystrokes between two uploads
// are one upload rather than ten.
//
// The size check lives here rather than in the composer because the limit is about encoded bytes
// and the frontend has no idea what a base64 part costs. It is an estimate on purpose: building the
// message means reading every attachment off the disk, which is not something to do on a keystroke,
// and the send builds the real thing and checks again.
//
// The upload does not go through `sync::outbox`. That queue is drained against `sync::Remote`,
// which is `Provider` narrowed to what the engine needs and carries `send` but neither `draft_put`
// nor `draft_delete`, so a draft row in it would be a row the drain cannot push and every flag
// change behind it would wait. Widening that trait is the sync package's file to widen; until then
// the queue is the `drafts` table, which already has the two columns it needs.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::decisions::db_of;
use crate::dto::{Draft, DraftAttachment, DraftSaved, Person};
use crate::mime::build::{self, Outgoing, OutgoingAttachment};
use crate::mirror::{read, write};
use crate::sync::{self, Store};

/// Gmail's ceiling on one message, encoded. Checked on every save so an attachment is refused while
/// it can still be taken off again, rather than at the send.
pub const MAX_ENCODED_BYTES: u64 = 35 * 1024 * 1024;

/// How often a draft may reach the provider. The composer's own debounce decides how often a save
/// arrives; this decides how often one of them leaves the machine.
pub const UPLOAD_EVERY_MS: i64 = 5_000;

/// What sits in the `drafts` row's payload.
///
/// The version is bumped on every save and is what says whether the provider is behind. A
/// millisecond is not fine enough to tell two keystrokes apart, and a clock is the wrong thing to
/// ask anyway: what matters is whether this is the copy that went, not when it went. `uploaded_at`
/// is only the rate limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    draft: Draft,
    #[serde(default)]
    version: i64,
    #[serde(default)]
    uploaded_version: i64,
    #[serde(default)]
    uploaded_at: i64,
}

// ---------------------------------------------------------------------------------------------
// The local copy
// ---------------------------------------------------------------------------------------------

fn stored(conn: &Connection, id: &str) -> Result<Option<(Stored, Option<String>, i64)>, String> {
    let held: Option<(String, Option<String>, i64)> = conn
        .query_row(
            "SELECT payload, provider_draft_id, updated_at FROM drafts WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((payload, provider_draft_id, updated_at)) = held else {
        return Ok(None);
    };
    let stored: Stored = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    Ok(Some((stored, provider_draft_id, updated_at)))
}

/// Writes the draft and answers with what the composer needs to know about its size.
///
/// The second save of a draft updates the row rather than adding one: the id is the app's own and
/// the composer sends it back with every save.
pub fn save(conn: &Connection, draft: &Draft) -> Result<DraftSaved, String> {
    let id = draft
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| write::fresh_id("draft"));
    let mut draft = draft.clone();
    draft.id = Some(id.clone());

    let held = stored(conn, &id)?.map(|(held, _, _)| held);
    let payload = serde_json::to_string(&Stored {
        draft: draft.clone(),
        version: held.as_ref().map(|held| held.version).unwrap_or(0) + 1,
        uploaded_version: held.as_ref().map(|held| held.uploaded_version).unwrap_or(0),
        uploaded_at: held.map(|held| held.uploaded_at).unwrap_or(0),
    })
    .map_err(|e| e.to_string())?;
    let now = write::now_ms();

    conn.execute(
        "INSERT INTO drafts (id, thread_key, in_reply_to, payload, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            thread_key = excluded.thread_key,
            in_reply_to = excluded.in_reply_to,
            payload = excluded.payload,
            updated_at = excluded.updated_at",
        rusqlite::params![id, draft.thread_key, draft.in_reply_to, payload, now],
    )
    .map_err(|e| e.to_string())?;

    let encoded_size = encoded_size(&draft);
    Ok(DraftSaved {
        id,
        updated_at_ms: now,
        encoded_size,
        over_limit: encoded_size > MAX_ENCODED_BYTES,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Draft, String> {
    stored(conn, id)?
        .map(|(held, _, _)| held.draft)
        .ok_or_else(|| "that draft is not on this device".to_string())
}

/// Removes the local row and hands back the provider's draft id, when the draft had reached it, so
/// the caller can take that copy away too.
pub fn delete(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    let provider_draft_id = stored(conn, id)?.and_then(|(_, provider, _)| provider);
    conn.execute("DELETE FROM drafts WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(provider_draft_id)
}

// ---------------------------------------------------------------------------------------------
// What it weighs
// ---------------------------------------------------------------------------------------------

/// Base64 is four characters for every three bytes, with a line ending every 76 of them.
fn base64_len(bytes: u64) -> u64 {
    let encoded = bytes.div_ceil(3) * 4;
    encoded + encoded.div_ceil(76) * 2
}

/// What this draft will weigh on the wire, near enough to refuse an attachment with.
///
/// Estimated rather than built. The body is counted twice because every message carries a plain
/// text alternative built from the HTML, and each part carries its own headers and boundary.
pub fn encoded_size(draft: &Draft) -> u64 {
    const PART_OVERHEAD: u64 = 512;

    let mut total = PART_OVERHEAD * 2 + draft.body_html.len() as u64 * 2;
    total += draft.subject.len() as u64;
    for person in draft.to.iter().chain(&draft.cc).chain(&draft.bcc) {
        total += person.address.len() as u64 + 32;
    }
    for attachment in &draft.attachments {
        total += base64_len(attachment.size) + PART_OVERHEAD + attachment.filename.len() as u64;
    }
    total
}

// ---------------------------------------------------------------------------------------------
// The bytes
// ---------------------------------------------------------------------------------------------

/// One attachment's bytes: a file the composer named, else a part of a message already here, which
/// is what a forward carries.
pub fn attachment_bytes(
    conn: &Connection,
    attachment: &DraftAttachment,
) -> Result<Vec<u8>, String> {
    if let Some(path) = attachment.path.as_deref().filter(|path| !path.is_empty()) {
        return std::fs::read(path)
            .map_err(|e| format!("{} could not be read: {e}", attachment.filename));
    }
    let id = attachment
        .attachment_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("{} has neither a file nor an attachment", attachment.filename))?;

    let row = read::attachment_row(conn, id)?
        .ok_or_else(|| format!("{} is not on this device", attachment.filename))?;
    if let Some(path) = &row.cached_path {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(bytes);
        }
    }
    let raw = read::raw_body(conn, &row.message_id)?
        .ok_or_else(|| format!("{} is not on this device", attachment.filename))?;
    part_bytes(&raw, &row.filename, row.size)
        .ok_or_else(|| format!("{} could not be read out of its message", attachment.filename))
}

/// The bytes of one part of a message on the device, matched on what the attachment row holds: the
/// decoded length and the file name. Body parts are skipped, so a text body the same length as a
/// file cannot be mistaken for it.
fn part_bytes(raw: &[u8], filename: &str, size: u64) -> Option<Vec<u8>> {
    use mail_parser::{MessageParser, MimeHeaders, PartType};

    let message = MessageParser::default().parse(raw)?;
    let bodies: HashSet<usize> = message
        .html_body
        .iter()
        .chain(message.text_body.iter())
        .map(|index| *index as usize)
        .collect();

    let mut fallback = None;
    for (index, part) in message.parts.iter().enumerate() {
        if bodies.contains(&index) {
            continue;
        }
        let bytes = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.to_vec(),
            PartType::Text(text) | PartType::Html(text) => text.as_bytes().to_vec(),
            PartType::Multipart(_) | PartType::Message(_) => continue,
        };
        if bytes.len() as u64 != size {
            continue;
        }
        if part.attachment_name() == Some(filename) {
            return Some(bytes);
        }
        fallback.get_or_insert(bytes);
    }
    fallback
}

/// The draft as RFC 2822 bytes. The threading headers are the caller's to supply, because what a
/// reply belongs to is a fact about the thread and not about the draft.
pub fn built(
    conn: &Connection,
    draft: &Draft,
    from: &Person,
    message_id: Option<&str>,
    in_reply_to: Option<&str>,
    references: &[String],
    subject: &str,
) -> Result<Vec<u8>, String> {
    let mut attachments = Vec::new();
    for attachment in &draft.attachments {
        attachments.push(OutgoingAttachment {
            filename: attachment.filename.clone(),
            mime_type: attachment.mime_type.clone(),
            bytes: attachment_bytes(conn, attachment)?,
        });
    }

    build::build(&Outgoing {
        from: from.clone(),
        to: draft.to.clone(),
        cc: draft.cc.clone(),
        bcc: draft.bcc.clone(),
        reply_to: Vec::new(),
        subject: subject.to_string(),
        html: draft.body_html.clone(),
        stylesheet: None,
        message_id: message_id.map(str::to_string),
        in_reply_to: in_reply_to.map(str::to_string),
        references: references.to_vec(),
        date_ms: Some(write::now_ms()),
        attachments,
        inline: Vec::new(),
    })
}

// ---------------------------------------------------------------------------------------------
// The copy that roams
// ---------------------------------------------------------------------------------------------

/// The drafts whose local copy is ahead of the provider's and whose moment has come.
fn due(conn: &Connection, now: i64) -> Result<Vec<(String, Option<String>, Stored)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, provider_draft_id, payload FROM drafts")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for (id, provider_draft_id, payload) in rows {
        let Ok(held) = serde_json::from_str::<Stored>(&payload) else {
            continue;
        };
        if held.version > held.uploaded_version && now - held.uploaded_at >= UPLOAD_EVERY_MS {
            out.push((id, provider_draft_id, held));
        }
    }
    Ok(out)
}

/// Records which version of the draft the provider now has. The payload is read back rather than
/// kept, because the composer may have written again while the upload was in the air and that write
/// must not be lost to this one.
fn uploaded(
    conn: &Connection,
    id: &str,
    provider_draft_id: &str,
    version: i64,
    now: i64,
) -> Result<(), String> {
    let Some((mut held, _, _)) = stored(conn, id)? else {
        return Ok(());
    };
    held.uploaded_version = version;
    held.uploaded_at = now;
    let payload = serde_json::to_string(&held).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE drafts SET provider_draft_id = ?2, payload = ?3 WHERE id = ?1",
        rusqlite::params![id, provider_draft_id, payload],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Uploads every draft that is due, and answers with how many went.
///
/// Generic over the provider rather than taking `sync::Remote`, because the two calls a draft makes
/// are the two that trait does not carry.
pub async fn upload<S: Store, P: sync::Remote + ?Sized>(
    store: &S,
    provider: &P,
    from: &Person,
    now: i64,
) -> Result<u32, String> {
    let mut sent = 0u32;
    for (id, provider_draft_id, held) in store.with(|conn| due(conn, now))? {
        let subject = held.draft.subject.clone();
        let from = crate::send::sender(&held.draft, from);
        let raw = store.with(|conn| built(conn, &held.draft, &from, None, None, &[], &subject))?;
        let thread_hint = store.with(|conn| thread_hint(conn, held.draft.thread_key.as_deref()))?;

        let put = provider
            .draft_put(provider_draft_id.as_deref(), &raw, thread_hint.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        store.with(|conn| uploaded(conn, &id, &put, held.version, now))?;
        sent += 1;
    }
    Ok(sent)
}

/// The provider's own id for a thread, so a draft or a reply lands in it on their side too.
pub fn thread_hint(conn: &Connection, thread_key: Option<&str>) -> Result<Option<String>, String> {
    let Some(key) = thread_key.filter(|key| !key.is_empty()) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT provider_thread_id FROM threads WHERE thread_key = ?1 ORDER BY latest_ms DESC
         LIMIT 1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// One account at a time, so two saves in the same second cannot both upload the same draft.
fn uploading() -> &'static Mutex<HashSet<String>> {
    static UPLOADING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    UPLOADING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Puts every draft of one account that is due in front of the provider.
///
/// Public because the poll loop is the other honest place to call it from: a draft written just
/// before the app was quit is uploaded when the app comes back, rather than waiting for somebody to
/// open the composer again.
pub async fn upload_pending(app: &tauri::AppHandle, account_id: &str) -> Result<u32, String> {
    if !uploading()
        .lock()
        .map(|mut held| held.insert(account_id.to_string()))
        .unwrap_or(false)
    {
        return Ok(0);
    }
    let done = upload_now(app, account_id).await;
    if let Ok(mut held) = uploading().lock() {
        held.remove(account_id);
    }
    done
}

async fn upload_now(app: &tauri::AppHandle, account_id: &str) -> Result<u32, String> {
    let db = db_of(app)?;
    let from = crate::send::own_person(app, db.inner(), account_id)?;
    let store = sync::Scoped {
        db: db.inner(),
        account_id,
    };
    // The handle the engine registered for this account, which is the only thing in this module
    // that knows the mailbox behind it is Gmail rather than an IMAP server.
    let Some(provider) = sync::remote_for(account_id) else {
        return Ok(0);
    };
    upload(&store, provider.as_ref(), &from, write::now_ms()).await
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn draft_save(app: tauri::AppHandle, draft: Draft) -> Result<DraftSaved, String> {
    let db = db_of(&app)?;
    let account_id = draft.account_id.clone();
    let saved = db.with(&account_id, |conn| save(conn, &draft))?;

    // The local write is the save. The upload is behind it on its own clock, so a composer that
    // saves on every debounce is not a client that uploads on every debounce.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(UPLOAD_EVERY_MS as u64)).await;
        let _ = upload_pending(&handle, &account_id).await;
    });
    Ok(saved)
}

#[tauri::command(async)]
pub fn draft_get(app: tauri::AppHandle, id: String) -> Result<Draft, String> {
    let db = db_of(&app)?;
    for (account_id, _) in sync::accounts(db.inner(), None) {
        if let Ok(draft) = db.with(&account_id, |conn| get(conn, &id)) {
            return Ok(draft);
        }
    }
    Err("that draft is not on this device".to_string())
}

#[tauri::command]
pub async fn draft_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let db = db_of(&app)?;
    for (account_id, _) in sync::accounts(db.inner(), None) {
        let held = db.with(&account_id, |conn| stored(conn, &id))?;
        if held.is_none() {
            continue;
        }
        let provider_draft_id = db.with(&account_id, |conn| delete(conn, &id))?;
        if let Some(provider_draft_id) = provider_draft_id {
            // The local row has gone either way. A provider that will not take the delete leaves a
            // draft in the mailbox's own Drafts, which is visible and fixable, and refusing the
            // command over it would leave the one on this machine that the person asked to be rid
            // of.
            if let Some(provider) = sync::remote_for(&account_id) {
                let _ = provider.draft_delete(&provider_draft_id).await;
            }
        }
        crate::emit_store_changed(&app, "threads");
        return Ok(());
    }
    Err("that draft is not on this device".to_string())
}

#[cfg(test)]
mod tests;
