// Clips, and the All files place.
//
// Two libraries built from what is already on the device. A clip is a sentence somebody kept, held
// in the state database with the thread and the sender it came from, so it survives the mirror
// being thrown away. All files is the other way round: it is entirely derived, it fetches nothing,
// and opening a card is what puts a byte on the network.
//
// The exclusion in `files_list` is the difference between a library and a wall of logos. Every
// newsletter carries a signature image, every one of them is inline and a few kilobytes, and a
// files place that lists them buries the contract somebody is looking for under two hundred of
// them.

use rusqlite::{params_from_iter, types::Value, Connection, OptionalExtension};

use crate::decisions::{account_holding, db_of};
use crate::dto::{Attachment, Clip, FileCard, Person, Undo};
use crate::state::{self, journal::Payload};
use crate::sync;
use crate::undo::Stack;

static UNDO: Stack<UndoClip> = Stack::new("clip");

/// Enough that nobody's library is truncated, and a ceiling rather than none at all.
const MAX_CLIPS: u32 = 5_000;

/// Signature junk, as `docs/features.md` section 10 defines it: an image under ten kilobytes that
/// the body refers to rather than lists.
const SIGNATURE_BYTES: u64 = 10 * 1024;

// ---------------------------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------------------------

fn clip_by_id(conn: &Connection, account_id: &str, id: &str) -> Result<Option<Clip>, String> {
    Ok(state::read::clips(conn, account_id, MAX_CLIPS)?
        .into_iter()
        .find(|clip| clip.id == id))
}

/// Who wrote the message the clip was taken from, and what the thread is called. Kept on the clip
/// rather than looked up later, because a clip outlives the message: the window moves on and the
/// card still has to say where the words came from.
fn provenance(conn: &Connection, message_id: &str) -> Result<(Person, String), String> {
    conn.query_row(
        "SELECT m.from_name, m.from_address, COALESCE(rn.name, m.subject)
           FROM messages m
           LEFT JOIN state.renames rn ON rn.thread_key = m.thread_key
          WHERE m.id = ?1",
        [message_id],
        |row| {
            Ok((
                Person {
                    name: row.get(0)?,
                    address: row.get(1)?,
                },
                row.get(2)?,
            ))
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "that message is not on this device".to_string())
}

pub fn save(
    conn: &Connection,
    account_id: &str,
    thread_key: &str,
    message_id: &str,
    text: &str,
) -> Result<Clip, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a clip needs some words in it".into());
    }
    let (sender, subject) = provenance(conn, message_id)?;
    let id = state::write::add_clip(conn, thread_key, message_id, text, &sender, &subject)?;
    clip_by_id(conn, account_id, &id)?.ok_or_else(|| "that clip did not land".to_string())
}

/// Puts a deleted clip back. Deletion is a flag rather than a missing row so that two devices
/// resolve it the same way, and that is what makes this possible.
fn restore(conn: &Connection, clip: &Clip) -> Result<(), String> {
    state::journal::append(
        conn,
        &clip.id,
        &Payload::Clip {
            thread_key: clip.thread_key.clone(),
            message_id: clip.message_id.clone(),
            text: clip.text.clone(),
            sender_name: clip.sender.name.clone(),
            sender_address: clip.sender.address.clone(),
            subject: clip.subject.clone(),
            created_at: clip.created_at_ms,
            deleted: false,
        },
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// All files
// ---------------------------------------------------------------------------------------------

fn extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default()
}

/// The eight buckets from section 10, decided from the MIME type first and the extension second.
///
/// Both, because neither is reliable on its own: half the world sends a spreadsheet as
/// `application/octet-stream`, and a file called `notes` with `text/calendar` on it is an invite.
/// The order matters in one place, which is that an invite is checked before a document, because
/// `text/calendar` would otherwise be caught as text.
pub fn category_of(mime_type: &str, filename: &str) -> &'static str {
    let mime = mime_type.trim().to_lowercase();
    let mime = mime.split(';').next().unwrap_or("").trim();
    let ext = extension(filename);

    if mime == "text/calendar" || mime == "application/ics" || ext == "ics" || ext == "ical" {
        return "invites";
    }
    if mime == "application/pdf" || ext == "pdf" {
        return "pdfs";
    }
    if mime.starts_with("image/")
        || matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "bmp" | "tiff" | "svg" | "avif"
        )
    {
        return "images";
    }
    if SPREADSHEET_TYPES.contains(&mime)
        || matches!(ext.as_str(), "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" | "numbers")
    {
        return "spreadsheets";
    }
    if PRESENTATION_TYPES.contains(&mime)
        || matches!(ext.as_str(), "ppt" | "pptx" | "odp" | "key")
    {
        return "presentations";
    }
    if ARCHIVE_TYPES.contains(&mime)
        || matches!(ext.as_str(), "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar")
    {
        return "archives";
    }
    if DOCUMENT_TYPES.contains(&mime)
        || mime.starts_with("text/")
        || matches!(ext.as_str(), "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "pages" | "epub")
    {
        return "documents";
    }
    "other"
}

const SPREADSHEET_TYPES: [&str; 4] = [
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.oasis.opendocument.spreadsheet",
    "text/csv",
];

const PRESENTATION_TYPES: [&str; 3] = [
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.presentation",
];

const ARCHIVE_TYPES: [&str; 8] = [
    "application/zip",
    "application/x-zip-compressed",
    "application/x-tar",
    "application/gzip",
    "application/x-gzip",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar-compressed",
];

const DOCUMENT_TYPES: [&str; 4] = [
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.oasis.opendocument.text",
    "application/rtf",
];

/// True for the images a signature block hangs off the bottom of a message.
fn signature_junk(attachment: &Attachment, category: &str) -> bool {
    category == "images" && attachment.inline && attachment.size < SIGNATURE_BYTES
}

/// Every file on the device as a card, newest first, filtered by type and by sender.
///
/// Trash and spam are left out. The place is a library and the mail is still there to be found in
/// Trash; a library that lists the attachments of deleted mail is a library nobody trusts.
pub fn files(conn: &Connection, category: &str, sender: &str) -> Result<Vec<FileCard>, String> {
    let chain = state::read::CHAIN;
    let sql = format!(
        "{chain}
         SELECT a.id, a.message_id, a.filename, a.mime_type, a.size, a.inline, a.content_id,
                a.cached_path IS NOT NULL AS cached,
                COALESCE((SELECT key FROM chain
                           WHERE source = t.thread_key
                             AND key NOT IN (SELECT thread_key FROM state.merges) LIMIT 1),
                         t.thread_key) AS thread_key,
                COALESCE(rn.name, t.subject) AS subject,
                m.from_name AS from_name, m.from_address AS from_address, m.date_ms AS date_ms
           FROM attachments a
           JOIN messages m ON m.id = a.message_id
           JOIN threads t ON t.provider_thread_id = m.provider_thread_id
           LEFT JOIN state.renames rn ON rn.thread_key = t.thread_key
          WHERE t.trashed = 0 AND t.spam = 0
            AND (? = '' OR lower(m.from_address) = ?)
          ORDER BY m.date_ms DESC, a.id ASC"
    );
    let sender = sender.trim().to_lowercase();
    let params = vec![Value::Text(sender.clone()), Value::Text(sender)];

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            Ok((
                Attachment {
                    id: row.get("id")?,
                    message_id: row.get("message_id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get::<_, i64>("size")?.max(0) as u64,
                    inline: row.get::<_, i64>("inline")? != 0,
                    content_id: row.get("content_id")?,
                    cached: row.get::<_, i64>("cached")? != 0,
                },
                row.get::<_, String>("thread_key")?,
                row.get::<_, String>("subject")?,
                Person {
                    name: row.get("from_name")?,
                    address: row.get("from_address")?,
                },
                row.get::<_, i64>("date_ms")?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let wanted = category.trim();
    let mut out = Vec::new();
    for (attachment, thread_key, subject, sender, date_ms) in rows {
        let category = category_of(&attachment.mime_type, &attachment.filename);
        if signature_junk(&attachment, category) {
            continue;
        }
        if !wanted.is_empty() && wanted != category {
            continue;
        }
        out.push(FileCard {
            attachment,
            thread_key,
            subject,
            sender,
            date_ms,
            category: category.to_string(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

pub struct UndoClip {
    pub account_id: String,
    pub clip: Clip,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that clip can no longer be brought back")?;
    let db = db_of(app)?;
    db.with(&entry.account_id, |conn| restore(conn, &entry.clip))?;
    crate::emit_store_changed(app, "state");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn clip_save(
    app: tauri::AppHandle,
    account_id: String,
    thread_key: String,
    message_id: String,
    text: String,
) -> Result<Clip, String> {
    let db = db_of(&app)?;
    let clip = db.with(&account_id, |conn| {
        save(conn, &account_id, &thread_key, &message_id, &text)
    })?;
    crate::emit_store_changed(&app, "state");
    Ok(clip)
}

#[tauri::command(async)]
pub fn clips_list(
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<Vec<Clip>, String> {
    let db = db_of(&app)?;
    let mut all = Vec::new();
    for (id, _) in sync::accounts(db.inner(), account_id.as_deref()) {
        all.extend(db.with(&id, |conn| state::read::clips(conn, &id, MAX_CLIPS))?);
    }
    all.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(b.id.cmp(&a.id)));
    Ok(all)
}

#[tauri::command(async)]
pub fn clip_delete(app: tauri::AppHandle, id: String) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let account_id = account_holding(
        db.inner(),
        "SELECT COUNT(*) FROM state.clips WHERE id = ?1 AND deleted = 0",
        &id,
    )
    .map_err(|_| "that clip is not here".to_string())?;
    let clip = db.with(&account_id, |conn| {
        let held = clip_by_id(conn, &account_id, &id)?
            .ok_or_else(|| "that clip is not here".to_string())?;
        state::write::delete_clip(conn, &id)?;
        Ok(held)
    })?;
    crate::emit_store_changed(&app, "state");

    let token = UNDO.push(UndoClip { account_id, clip }, "Clip deleted");
    Ok(Undo {
        token,
        label: "Clip deleted".to_string(),
        undo_ms: 0,
    })
}

/// The All files place. Built from the local index and fetches nothing: the card knows the name,
/// the type, the size, the sender and the thread without a byte leaving the machine.
#[tauri::command(async)]
pub fn files_list(
    app: tauri::AppHandle,
    account_id: Option<String>,
    category: String,
    sender: String,
) -> Result<Vec<FileCard>, String> {
    let db = db_of(&app)?;
    let mut all = Vec::new();
    for (id, _) in sync::accounts(db.inner(), account_id.as_deref()) {
        all.extend(db.with(&id, |conn| files(conn, &category, &sender))?);
    }
    all.sort_by(|a, b| {
        b.date_ms
            .cmp(&a.date_ms)
            .then(a.attachment.id.cmp(&b.attachment.id))
    });
    Ok(all)
}

#[cfg(test)]
mod tests;
