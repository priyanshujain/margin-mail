// Everything that puts a row in the mirror.
//
// Two facts are derived here and kept as columns rather than worked out on read: the portable
// thread key, and the routing headers. Both are read on every row of a list page, and a JSON
// extract per row over a first sync is the difference between a second and a minute.
//
// The system label names below are the one place in the app outside the provider module that
// knows what a Gmail label is called. `RawHeaders` carries `label_ids` and nothing else, so the
// seen, starred, archived, trashed and spam flags have nowhere else to come from. An IMAP
// provider maps `\Seen` and `X-GM-LABELS` onto the same names before the mirror sees them.

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::dto::{FlagPatch, Person};
use crate::mime::{self, RenderOptions};
use crate::provider::RawHeaders;

pub const LABEL_INBOX: &str = "INBOX";
pub const LABEL_UNREAD: &str = "UNREAD";
pub const LABEL_STARRED: &str = "STARRED";
pub const LABEL_TRASH: &str = "TRASH";
pub const LABEL_SPAM: &str = "SPAM";
pub const LABEL_SENT: &str = "SENT";
pub const LABEL_DRAFT: &str = "DRAFT";

pub const CURSOR_KEY: &str = "sync-cursor";
pub const LAST_SYNC_KEY: &str = "last-sync";
pub const WINDOW_KEY: &str = "window-days";
pub const BACKFILL_KEY: &str = "backfill-after";
pub const OWN_ADDRESS_KEY: &str = "own-address";
pub const ACCOUNT_COLOR_KEY: &str = "account-color";
pub const FIRST_SYNC_KEY: &str = "first-sync-done";

/// The default window, in days. Thirty days is what a mail client is actually used for.
pub const DEFAULT_WINDOW_DAYS: i64 = 30;

pub const DAY_MS: i64 = 86_400_000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Unique within the process and monotonic within a millisecond, which is all an outbox row or an
/// undo token needs. There is no uuid dependency here and adding one to name a queue entry would
/// be a poor trade.
pub fn fresh_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{prefix}-{}-{}", now_ms(), NEXT.fetch_add(1, Ordering::Relaxed))
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn meta_i64(conn: &Connection, key: &str) -> Result<Option<i64>, String> {
    Ok(meta_get(conn, key)?.and_then(|v| v.parse().ok()))
}

pub fn meta_clear(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// How far back this account keeps mail, in days. Zero is everything.
pub fn window_days(conn: &Connection) -> Result<i64, String> {
    Ok(meta_i64(conn, WINDOW_KEY)?.unwrap_or(DEFAULT_WINDOW_DAYS))
}

/// The moment the window starts, or None when the account keeps everything.
pub fn window_start(conn: &Connection, now: i64) -> Result<Option<i64>, String> {
    let days = window_days(conn)?;
    Ok((days > 0).then(|| now - days * DAY_MS))
}

// ---------------------------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------------------------

/// A `Message-ID` without its angle brackets, which is the form the state database keys on.
pub fn bare_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The first entry of a `References` or `In-Reply-To` header. Folding has already been undone by
/// the provider, so the entries are separated by whitespace.
pub fn first_reference(raw: &str) -> Option<String> {
    raw.split_whitespace().find_map(bare_id)
}

/// The portable thread key: the first entry of `References`, else `In-Reply-To`, else the
/// message's own `Message-ID`.
///
/// The fallback is the provider's thread id, which is the honest failure: a message with no
/// `Message-ID` at all cannot have a key that survives a change of provider, and borrowing the
/// provider's id at least keeps the conversation together on this device.
pub fn thread_key(
    references: Option<&str>,
    in_reply_to: Option<&str>,
    message_id: Option<&str>,
    provider_thread_id: &str,
) -> String {
    references
        .and_then(first_reference)
        .or_else(|| in_reply_to.and_then(first_reference))
        .or_else(|| message_id.and_then(bare_id))
        .unwrap_or_else(|| format!("provider:{provider_thread_id}"))
}

/// Splits an address list on the commas that are not inside a quoted display name or a set of
/// angle brackets. Addresses are lowercased because every lookup in the app, from a sender rule to
/// a contact card, treats them case-insensitively.
pub fn addresses(header: &str) -> Vec<Person> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut depth = 0i32;
    for c in header.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            '<' if !quoted => {
                depth += 1;
                current.push(c);
            }
            '>' if !quoted => {
                depth -= 1;
                current.push(c);
            }
            ',' if !quoted && depth <= 0 => {
                push_person(&mut out, &current);
                current.clear();
            }
            _ => current.push(c),
        }
    }
    push_person(&mut out, &current);
    out
}

fn push_person(out: &mut Vec<Person>, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    let (name, address) = match (raw.find('<'), raw.rfind('>')) {
        (Some(open), Some(close)) if close > open => (
            raw[..open].trim().to_string(),
            raw[open + 1..close].trim().to_string(),
        ),
        _ => (String::new(), raw.to_string()),
    };
    let name = name.trim().trim_matches('"').trim().to_string();
    if address.is_empty() {
        return;
    }
    out.push(Person {
        name: (!name.is_empty()).then_some(name),
        address: address.to_lowercase(),
    });
}

pub fn one_address(header: &str) -> Person {
    addresses(header).into_iter().next().unwrap_or(Person {
        name: None,
        address: String::new(),
    })
}

fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[]".to_string())
}

// ---------------------------------------------------------------------------------------------
// Messages and threads
// ---------------------------------------------------------------------------------------------

fn has_label(labels: &[String], name: &str) -> bool {
    labels.iter().any(|l| l == name)
}

/// A row from `messages.list` that has not been hydrated yet. The thread key is a placeholder
/// until the headers arrive, because there is nothing to derive one from.
pub fn note_listed(conn: &Connection, id: &str, provider_thread_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO messages (id, provider_thread_id, thread_key, hydrated)
         VALUES (?1, ?2, ?3, 0)",
        params![id, provider_thread_id, format!("provider:{provider_thread_id}")],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Ids waiting for their metadata, oldest listing first. The listing arrives newest first, so
/// insertion order is hydration order and the newest mail lands first.
pub fn unhydrated(conn: &Connection, limit: usize) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE hydrated = 0 ORDER BY rowid LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn count(conn: &Connection, sql: &str) -> Result<u32, String> {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .map(|n| n as u32)
        .map_err(|e| e.to_string())
}

/// Upserts one message from its metadata and returns the thread key it derived.
pub fn upsert_message(
    conn: &Connection,
    headers: &RawHeaders,
    transient: bool,
) -> Result<String, String> {
    let message_id = headers.header("message-id").and_then(bare_id);
    let in_reply_to = headers.header("in-reply-to").and_then(first_reference);
    let references_first = headers.header("references").and_then(first_reference);
    let key = thread_key(
        headers.header("references"),
        headers.header("in-reply-to"),
        headers.header("message-id"),
        &headers.thread_id,
    );

    let from = headers.header("from").map(one_address).unwrap_or(Person {
        name: None,
        address: String::new(),
    });
    let to = headers.header("to").map(addresses).unwrap_or_default();
    let cc = headers.header("cc").map(addresses).unwrap_or_default();
    let bcc = headers.header("bcc").map(addresses).unwrap_or_default();
    let reply_to = headers.header("reply-to").map(addresses).unwrap_or_default();

    let labels = &headers.label_ids;
    // Metadata carries no part list, so this is a guess off `Content-Type` and it is corrected the
    // moment the body lands and the real parts are known.
    let has_attachment = headers
        .header("content-type")
        .map(|value| value.to_ascii_lowercase().contains("multipart/mixed"))
        .unwrap_or(false);

    conn.execute(
        "INSERT INTO messages (
            id, provider_thread_id, thread_key, message_id, in_reply_to, references_first,
            date_ms, from_name, from_address, to_json, cc_json, bcc_json, reply_to_json,
            subject, snippet, seen, starred, draft, sent, labels,
            list_id, list_unsubscribe, list_unsub_post, auto_submitted, precedence,
            size, has_attachment, hydrated, transient
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20,
            ?21, ?22, ?23, ?24, ?25,
            ?26, ?27, 1, ?28
         )
         ON CONFLICT(id) DO UPDATE SET
            provider_thread_id = excluded.provider_thread_id,
            thread_key = excluded.thread_key,
            message_id = excluded.message_id,
            in_reply_to = excluded.in_reply_to,
            references_first = excluded.references_first,
            date_ms = excluded.date_ms,
            from_name = excluded.from_name,
            from_address = excluded.from_address,
            to_json = excluded.to_json,
            cc_json = excluded.cc_json,
            bcc_json = excluded.bcc_json,
            reply_to_json = excluded.reply_to_json,
            subject = excluded.subject,
            snippet = excluded.snippet,
            seen = excluded.seen,
            starred = excluded.starred,
            draft = excluded.draft,
            sent = excluded.sent,
            labels = excluded.labels,
            list_id = excluded.list_id,
            list_unsubscribe = excluded.list_unsubscribe,
            list_unsub_post = excluded.list_unsub_post,
            auto_submitted = excluded.auto_submitted,
            precedence = excluded.precedence,
            size = excluded.size,
            has_attachment = messages.has_attachment OR excluded.has_attachment,
            hydrated = 1,
            transient = messages.transient AND excluded.transient",
        params![
            headers.id,
            headers.thread_id,
            key,
            message_id,
            in_reply_to,
            references_first,
            headers.internal_date_ms,
            from.name,
            from.address,
            json(&to),
            json(&cc),
            json(&bcc),
            json(&reply_to),
            headers.header("subject").unwrap_or("").trim(),
            headers.snippet,
            !has_label(labels, LABEL_UNREAD) as i64,
            has_label(labels, LABEL_STARRED) as i64,
            has_label(labels, LABEL_DRAFT) as i64,
            has_label(labels, LABEL_SENT) as i64,
            json(labels),
            headers.header("list-id"),
            headers.header("list-unsubscribe"),
            headers.header("list-unsubscribe-post"),
            headers.header("auto-submitted"),
            headers.header("precedence"),
            headers.size as i64,
            has_attachment as i64,
            transient as i64,
        ],
    )
    .map_err(|e| e.to_string())?;

    remember_correspondents(conn, &from, &to, headers.internal_date_ms, has_label(labels, LABEL_SENT))?;
    // Indexed on its headers now rather than when a body arrives, because a body may never arrive:
    // the hits of a provider search are metadata only, and a result list is a query over this
    // index. `store_body` writes the row again with the text once there is text.
    super::fts::index(conn, &headers.id)?;
    Ok(key)
}

fn remember_correspondents(
    conn: &Connection,
    from: &Person,
    to: &[Person],
    date_ms: i64,
    sent: bool,
) -> Result<(), String> {
    let mut people: Vec<(&Person, bool)> = vec![(from, false)];
    people.extend(to.iter().map(|person| (person, sent)));
    for (person, outgoing) in people {
        if person.address.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO correspondents (address, name, last_ms, seen_count, sent_count, source)
             VALUES (?1, ?2, ?3, 1, ?4, 'mirror')
             ON CONFLICT(address) DO UPDATE SET
                name = COALESCE(excluded.name, correspondents.name),
                last_ms = MAX(correspondents.last_ms, excluded.last_ms),
                seen_count = correspondents.seen_count + 1,
                sent_count = correspondents.sent_count + ?4",
            params![person.address, person.name, date_ms, outgoing as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Recomputes one thread's denormalised columns from its messages. Called once per hydration batch
/// rather than once per message, because a thread's aggregate is the same either way and reading
/// every message of a thread per message written is quadratic.
pub fn refresh_thread(conn: &Connection, provider_thread_id: &str) -> Result<(), String> {
    struct Row {
        thread_key: String,
        date_ms: i64,
        subject: String,
        snippet: String,
        from_name: Option<String>,
        from_address: String,
        to_json: String,
        seen: bool,
        starred: bool,
        draft: bool,
        has_attachment: bool,
        labels: String,
        transient: bool,
    }

    let mut stmt = conn
        .prepare(
            "SELECT thread_key, date_ms, subject, snippet, from_name, from_address, to_json,
                    seen, starred, draft, has_attachment, labels, transient
             FROM messages
             WHERE provider_thread_id = ?1 AND hydrated = 1
             ORDER BY date_ms ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([provider_thread_id], |row| {
            Ok(Row {
                thread_key: row.get(0)?,
                date_ms: row.get(1)?,
                subject: row.get(2)?,
                snippet: row.get(3)?,
                from_name: row.get(4)?,
                from_address: row.get(5)?,
                to_json: row.get(6)?,
                seen: row.get::<_, i64>(7)? != 0,
                starred: row.get::<_, i64>(8)? != 0,
                draft: row.get::<_, i64>(9)? != 0,
                has_attachment: row.get::<_, i64>(10)? != 0,
                labels: row.get(11)?,
                transient: row.get::<_, i64>(12)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        conn.execute(
            "DELETE FROM threads WHERE provider_thread_id = ?1",
            [provider_thread_id],
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let latest = rows.last().expect("a non-empty thread");
    let mut participants: Vec<Person> = Vec::new();
    let mut in_inbox = false;
    let mut trashed = false;
    let mut spam = false;
    let mut unseen = false;
    let mut starred = false;
    let mut has_attachment = false;
    let mut has_draft = false;
    let mut transient = true;
    for row in &rows {
        let labels: Vec<String> = serde_json::from_str(&row.labels).unwrap_or_default();
        in_inbox |= has_label(&labels, LABEL_INBOX);
        trashed |= has_label(&labels, LABEL_TRASH);
        spam |= has_label(&labels, LABEL_SPAM);
        unseen |= !row.seen && !row.draft;
        starred |= row.starred;
        has_attachment |= row.has_attachment;
        has_draft |= row.draft;
        transient &= row.transient;
        let from = Person {
            name: row.from_name.clone(),
            address: row.from_address.clone(),
        };
        merge_person(&mut participants, from);
        for person in serde_json::from_str::<Vec<Person>>(&row.to_json).unwrap_or_default() {
            merge_person(&mut participants, person);
        }
    }

    conn.execute(
        "INSERT INTO threads (
            provider_thread_id, thread_key, latest_ms, message_count, unseen, starred,
            has_attachment, has_draft, in_inbox, trashed, spam, subject, snippet,
            from_name, from_address, participants, transient
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(provider_thread_id) DO UPDATE SET
            thread_key = excluded.thread_key,
            latest_ms = excluded.latest_ms,
            message_count = excluded.message_count,
            unseen = excluded.unseen,
            starred = excluded.starred,
            has_attachment = excluded.has_attachment,
            has_draft = excluded.has_draft,
            in_inbox = excluded.in_inbox,
            trashed = excluded.trashed,
            spam = excluded.spam,
            subject = excluded.subject,
            snippet = excluded.snippet,
            from_name = excluded.from_name,
            from_address = excluded.from_address,
            participants = excluded.participants,
            transient = excluded.transient",
        params![
            provider_thread_id,
            rows[0].thread_key,
            latest.date_ms,
            rows.len() as i64,
            unseen as i64,
            starred as i64,
            has_attachment as i64,
            has_draft as i64,
            in_inbox as i64,
            trashed as i64,
            spam as i64,
            rows.iter().find(|r| !r.subject.is_empty()).map(|r| r.subject.clone()).unwrap_or_default(),
            latest.snippet,
            latest.from_name,
            latest.from_address,
            json(&participants),
            transient as i64,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn merge_person(into: &mut Vec<Person>, person: Person) {
    if person.address.is_empty() {
        return;
    }
    match into.iter_mut().find(|held| held.address == person.address) {
        Some(held) => {
            if held.name.is_none() {
                held.name = person.name;
            }
        }
        None => into.push(person),
    }
}

pub fn thread_id_of(conn: &Connection, message_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT provider_thread_id FROM messages WHERE id = ?1",
        [message_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// The provider ids of every message in the threads a set of portable keys names, including the
/// threads merged into them.
pub fn message_ids_for_keys(conn: &Connection, keys: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for key in keys {
        let mut stmt = conn
            .prepare(
                "SELECT m.id FROM messages m
                 WHERE m.provider_thread_id IN (
                    SELECT provider_thread_id FROM threads
                    WHERE thread_key = ?1
                       OR thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1)
                 )",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        out.extend(ids);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// What a message's flags were before a change, which is what an undo needs and what a frontend
/// cannot reconstruct after a bulk action over mixed prior state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorFlags {
    pub id: String,
    pub labels: Vec<String>,
}

pub fn prior_flags(conn: &Connection, ids: &[String]) -> Result<Vec<PriorFlags>, String> {
    let mut out = Vec::new();
    for id in ids {
        let labels: Option<String> = conn
            .query_row("SELECT labels FROM messages WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(labels) = labels {
            out.push(PriorFlags {
                id: id.clone(),
                labels: serde_json::from_str(&labels).unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// The label a flag maps onto, and whether setting the flag adds it. Seen and archived are both
/// inversions, which is the sort of thing worth having in exactly one place.
fn label_for(flag: &str) -> (&'static str, bool) {
    match flag {
        "seen" => (LABEL_UNREAD, false),
        "starred" => (LABEL_STARRED, true),
        "archived" => (LABEL_INBOX, false),
        "trashed" => (LABEL_TRASH, true),
        "spam" => (LABEL_SPAM, true),
        _ => ("", true),
    }
}

fn apply_to_labels(labels: &mut Vec<String>, patch: &FlagPatch) {
    for (flag, value) in [
        ("seen", patch.seen),
        ("starred", patch.starred),
        ("archived", patch.archived),
        ("trashed", patch.trashed),
        ("spam", patch.spam),
    ] {
        let Some(on) = value else { continue };
        let (label, adds_when_on) = label_for(flag);
        if label.is_empty() {
            continue;
        }
        let should_hold = if adds_when_on { on } else { !on };
        let held = labels.iter().any(|l| l == label);
        if should_hold && !held {
            labels.push(label.to_string());
        } else if !should_hold && held {
            labels.retain(|l| l != label);
        }
    }
}

/// Lands a flag change in the mirror. The push to the provider is queued separately, because the
/// write is optimistic: it shows before it goes.
pub fn apply_flags(
    conn: &Connection,
    ids: &[String],
    patch: &FlagPatch,
) -> Result<Vec<String>, String> {
    let mut threads = Vec::new();
    for id in ids {
        let held: Option<(String, String)> = conn
            .query_row(
                "SELECT labels, provider_thread_id FROM messages WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((labels, thread_id)) = held else {
            continue;
        };
        let mut labels: Vec<String> = serde_json::from_str(&labels).unwrap_or_default();
        apply_to_labels(&mut labels, patch);
        set_labels_row(conn, id, &labels)?;
        if !threads.contains(&thread_id) {
            threads.push(thread_id);
        }
    }
    for thread_id in &threads {
        refresh_thread(conn, thread_id)?;
    }
    Ok(threads)
}

/// Lands the label set the provider reported for a message. An undo wants `apply_labels` instead,
/// for the reason below.
///
/// What the provider says goes under whatever this device has queued and not yet pushed. A sync
/// pass that fetched its metadata before an open landed still carries `UNREAD` for the thread, and
/// writing that would put the dot back for the twelve seconds until the outbox row it crossed
/// reaches the server; an archive and `INBOX` are the same race. Mailspring's rule, and the honest
/// one: a change this device made is the truth about those labels until the server has heard it.
pub fn set_labels(conn: &Connection, id: &str, labels: &[String]) -> Result<Option<String>, String> {
    let thread_id = thread_id_of(conn, id)?;
    if thread_id.is_some() {
        let labels = under_pending(conn, id, labels)?;
        set_labels_row(conn, id, &labels)?;
    }
    Ok(thread_id)
}

/// A reported label set with every change still waiting in the outbox for this message replayed
/// over it, oldest row first so that a later change wins the way it did when it was made. An
/// empty outbox, which is nearly always, is one query over an empty table and the set unchanged.
fn under_pending(conn: &Connection, id: &str, labels: &[String]) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT op, payload FROM outbox
             WHERE op IN (?1, ?2)
               AND EXISTS (SELECT 1 FROM json_each(outbox.payload, '$.ids') WHERE json_each.value = ?3)
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![OP_FLAGS, OP_LABELS, id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut labels = labels.to_vec();
    for (op, payload) in rows {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let field = |name: &str| payload.get(name).cloned().unwrap_or(serde_json::Value::Null);
        if op == OP_FLAGS {
            if let Ok(patch) = serde_json::from_value::<FlagPatch>(field("patch")) {
                apply_to_labels(&mut labels, &patch);
            }
        } else {
            let add: Vec<String> = serde_json::from_value(field("add")).unwrap_or_default();
            let remove: Vec<String> = serde_json::from_value(field("remove")).unwrap_or_default();
            for label in add {
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            labels.retain(|l| !remove.contains(l));
        }
    }
    Ok(labels)
}

fn set_labels_row(conn: &Connection, id: &str, labels: &[String]) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET labels = ?2, seen = ?3, starred = ?4, draft = ?5, sent = ?6
         WHERE id = ?1",
        params![
            id,
            json(&labels.to_vec()),
            !has_label(labels, LABEL_UNREAD) as i64,
            has_label(labels, LABEL_STARRED) as i64,
            has_label(labels, LABEL_DRAFT) as i64,
            has_label(labels, LABEL_SENT) as i64,
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Records a label change locally. Returns the threads that need refreshing.
pub fn apply_labels(
    conn: &Connection,
    ids: &[String],
    add: &[String],
    remove: &[String],
) -> Result<Vec<String>, String> {
    let mut threads = Vec::new();
    for id in ids {
        let held: Option<(String, String)> = conn
            .query_row(
                "SELECT labels, provider_thread_id FROM messages WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((labels, thread_id)) = held else {
            continue;
        };
        let mut labels: Vec<String> = serde_json::from_str(&labels).unwrap_or_default();
        for label in add {
            if !labels.contains(label) {
                labels.push(label.clone());
            }
        }
        labels.retain(|l| !remove.contains(l));
        set_labels_row(conn, id, &labels)?;
        if !threads.contains(&thread_id) {
            threads.push(thread_id);
        }
    }
    for thread_id in &threads {
        refresh_thread(conn, thread_id)?;
    }
    Ok(threads)
}

pub fn upsert_labels(
    conn: &Connection,
    labels: &[crate::provider::ProviderLabel],
) -> Result<(), String> {
    for label in labels {
        conn.execute(
            "INSERT INTO labels (id, name, kind) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, kind = excluded.kind",
            params![label.id, label.name, label.kind],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Bodies and attachments
// ---------------------------------------------------------------------------------------------

/// Stores a body and whatever the pipeline could make of it.
///
/// A body that will not render is still stored, with its render left empty and its version left at
/// zero, so the thread opens with its headers rather than not at all. `RENDER_VERSION` on the row
/// is what makes that recoverable: the next open of a message whose stamp is not the current one
/// renders it again from the raw bytes it kept.
/// The first line or so of a message, for the list row.
///
/// Whitespace is collapsed because a plain text body arrives hard wrapped and a preview with the
/// original line breaks in it renders as one long line with gaps in the middle. The cut is on a
/// word boundary when there is one within reach of the limit, because a snippet ending mid-word
/// reads as a rendering fault rather than as a preview.
pub fn preview(text: &str) -> String {
    const LIMIT: usize = 200;

    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LIMIT {
        return collapsed;
    }

    let cut = collapsed
        .char_indices()
        .nth(LIMIT)
        .map(|(at, _)| at)
        .unwrap_or(collapsed.len());
    let head = &collapsed[..cut];
    match head.rfind(' ') {
        // Only when the last space is near enough that trimming to it is a word boundary rather
        // than throwing away half the preview.
        Some(space) if space > cut * 3 / 4 => head[..space].to_string(),
        _ => head.to_string(),
    }
}

/// The stored form of a surface. A word rather than a flag, because a third answer is a plausible
/// thing to want later and a boolean column that has to grow one is a migration.
fn surface_name(surface: crate::dto::Surface) -> &'static str {
    match surface {
        crate::dto::Surface::Theme => "theme",
        crate::dto::Surface::Paper => "paper",
    }
}

pub fn store_body(
    conn: &Connection,
    message_id: &str,
    raw: &[u8],
    options: &RenderOptions,
) -> Result<(), String> {
    let rendered = mime::render(raw, options);
    let now = now_ms();
    match rendered {
        Ok(body) => {
            conn.execute(
                "INSERT INTO bodies (message_id, raw, html, quoted_html, text, is_html, surface,
                                     trackers, blocked_images, render_version, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(message_id) DO UPDATE SET
                    raw = excluded.raw,
                    html = excluded.html,
                    quoted_html = excluded.quoted_html,
                    text = excluded.text,
                    is_html = excluded.is_html,
                    surface = excluded.surface,
                    trackers = excluded.trackers,
                    blocked_images = excluded.blocked_images,
                    render_version = excluded.render_version,
                    fetched_at = excluded.fetched_at",
                params![
                    message_id,
                    raw,
                    body.html,
                    body.quoted_html,
                    body.text,
                    body.is_html as i64,
                    surface_name(body.surface),
                    json(&body.trackers),
                    body.blocked_images as i64,
                    mime::RENDER_VERSION,
                    now,
                ],
            )
            .map_err(|e| e.to_string())?;
            store_attachments(conn, message_id, &body.attachments)?;
            // The snippet is only written when there is not one already. Gmail sends its own with
            // the metadata and that one is better: it is what Gmail itself shows, so a thread does
            // not change its preview line the first time somebody opens it. A provider with no
            // snippet of its own, which is every IMAP server, gets one from the body instead, and
            // this is the only moment the text exists to take it from.
            conn.execute(
                "UPDATE messages
                    SET has_attachment = ?2,
                        snippet = CASE WHEN snippet = '' THEN ?3 ELSE snippet END
                  WHERE id = ?1",
                params![
                    message_id,
                    body.attachments.iter().any(|a| !a.inline) as i64,
                    preview(body.text.as_deref().unwrap_or_default())
                ],
            )
            .map_err(|e| e.to_string())?;
            if let Some(thread_id) = thread_id_of(conn, message_id)? {
                refresh_thread(conn, &thread_id)?;
            }
        }
        Err(_) => {
            conn.execute(
                "INSERT INTO bodies (message_id, raw, html, render_version, fetched_at)
                 VALUES (?1, ?2, '', 0, ?3)
                 ON CONFLICT(message_id) DO UPDATE SET
                    raw = excluded.raw,
                    render_version = 0,
                    fetched_at = excluded.fetched_at",
                params![message_id, raw, now],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    super::fts::index(conn, message_id)
}

/// Renders again from the bytes already held, for a body whose cached render was made by an older
/// sanitiser or by no sanitiser at all.
pub fn rerender(
    conn: &Connection,
    message_id: &str,
    options: &RenderOptions,
) -> Result<bool, String> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT raw FROM bodies WHERE message_id = ?1 AND render_version <> ?2 AND raw IS NOT NULL",
            params![message_id, mime::RENDER_VERSION],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(raw) = raw else { return Ok(false) };
    store_body(conn, message_id, &raw, options)?;
    Ok(true)
}

pub fn store_attachments(
    conn: &Connection,
    message_id: &str,
    attachments: &[mime::RenderedAttachment],
) -> Result<(), String> {
    conn.execute("DELETE FROM attachments WHERE message_id = ?1", [message_id])
        .map_err(|e| e.to_string())?;
    for part in attachments {
        conn.execute(
            "INSERT INTO attachments (id, message_id, part_id, filename, mime_type, size, inline,
                                      content_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                filename = excluded.filename,
                mime_type = excluded.mime_type,
                size = excluded.size,
                inline = excluded.inline,
                content_id = excluded.content_id",
            params![
                format!("{message_id}:{}", part.part_id),
                message_id,
                part.part_id,
                part.filename,
                part.mime_type,
                part.size as i64,
                part.inline as i64,
                part.content_id,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Notes where the cache put an attachment's bytes.
pub fn attachment_cached(
    conn: &Connection,
    id: &str,
    path: &str,
    at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET cached_path = ?2, cached_at = ?3 WHERE id = ?1",
        params![id, path, at],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Forgets a cached file. The row stays: an attachment that is no longer on the disk is still an
/// attachment, and `storage_used` counts what the cache is holding rather than what a message has.
pub fn attachment_uncached(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET cached_path = NULL, cached_at = NULL WHERE id = ?1",
        [id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn delete_message(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    let thread_id = thread_id_of(conn, id)?;
    super::fts::remove(conn, id)?;
    for sql in [
        "DELETE FROM bodies WHERE message_id = ?1",
        "DELETE FROM attachments WHERE message_id = ?1",
        "DELETE FROM messages WHERE id = ?1",
    ] {
        conn.execute(sql, [id]).map_err(|e| e.to_string())?;
    }
    if let Some(thread_id) = &thread_id {
        refresh_thread(conn, thread_id)?;
    }
    Ok(thread_id)
}

// ---------------------------------------------------------------------------------------------
// The outbox
// ---------------------------------------------------------------------------------------------

pub const OP_FLAGS: &str = "flags";
pub const OP_LABELS: &str = "labels";
/// The send pipeline is a later package. A send is an outbox row like any other, with `hold_until`
/// carrying its undo delay, and `sync::outbox` refuses it with a readable error until that package
/// lands rather than pretending it went.
pub const OP_SEND: &str = "send";

#[derive(Debug, Clone, Default)]
pub struct OutboxRow {
    pub id: String,
    pub op: String,
    pub payload: String,
    pub thread_key: Option<String>,
    pub hold_until: i64,
    pub attempts: i64,
    pub created_at: i64,
    pub last_error: Option<String>,
}

pub fn enqueue(conn: &Connection, row: &OutboxRow) -> Result<String, String> {
    let id = if row.id.is_empty() {
        fresh_id("out")
    } else {
        row.id.clone()
    };
    conn.execute(
        "INSERT INTO outbox (id, op, payload, thread_key, hold_until, attempts, created_at,
                             last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            payload = excluded.payload,
            hold_until = excluded.hold_until,
            last_error = excluded.last_error",
        params![
            id,
            row.op,
            row.payload,
            row.thread_key,
            row.hold_until,
            row.attempts,
            if row.created_at == 0 { now_ms() } else { row.created_at },
            row.last_error,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn outbox_rows(conn: &Connection, limit: usize) -> Result<Vec<OutboxRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, op, payload, thread_key, hold_until, attempts, created_at, last_error
             FROM outbox ORDER BY created_at, id LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(OutboxRow {
                id: row.get(0)?,
                op: row.get(1)?,
                payload: row.get(2)?,
                thread_key: row.get(3)?,
                hold_until: row.get(4)?,
                attempts: row.get(5)?,
                created_at: row.get(6)?,
                last_error: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The newest row of a kind that has not been attempted and is not being held, which is the row a
/// second change of the same kind coalesces into.
pub fn coalescable(conn: &Connection, op: &str, now: i64) -> Result<Option<OutboxRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, op, payload, thread_key, hold_until, attempts, created_at, last_error
             FROM outbox
             WHERE op = ?1 AND attempts = 0 AND hold_until <= ?2
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![op, now], |row| {
            Ok(OutboxRow {
                id: row.get(0)?,
                op: row.get(1)?,
                payload: row.get(2)?,
                thread_key: row.get(3)?,
                hold_until: row.get(4)?,
                attempts: row.get(5)?,
                created_at: row.get(6)?,
                last_error: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(row) => row.map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub fn dequeue(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM outbox WHERE id = ?1", [id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn defer(conn: &Connection, id: &str, until: i64, error: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE outbox SET attempts = attempts + 1, hold_until = ?2, last_error = ?3 WHERE id = ?1",
        params![id, until, error],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn pending_writes(conn: &Connection) -> Result<u32, String> {
    count(conn, "SELECT COUNT(*) FROM outbox")
}

/// Throws the mirror away, leaving the state database untouched. The next sync fills it again.
pub fn clear(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DELETE FROM search;
         DELETE FROM bodies;
         DELETE FROM attachments;
         DELETE FROM messages;
         DELETE FROM threads;
         DELETE FROM correspondents;
         DELETE FROM meta WHERE key IN ('sync-cursor', 'last-sync', 'backfill-after',
                                        'first-sync-done');",
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_collapses_the_hard_wrapping_a_plain_text_body_arrives_with() {
        let wrapped = "Hi Ana,\n\nThe piano lesson moved to five.\n  See you then.\n";
        assert_eq!(
            preview(wrapped),
            "Hi Ana, The piano lesson moved to five. See you then."
        );
        assert_eq!(preview(""), "");
        assert_eq!(preview("   \n\t  "), "");
    }

    #[test]
    fn a_long_preview_is_cut_on_a_word_boundary_when_one_is_within_reach() {
        let long = "alpha ".repeat(60);
        let cut = preview(&long);
        assert!(cut.chars().count() <= 200);
        // On a boundary, so the last word is whole rather than sliced in half.
        assert!(cut.ends_with("alpha"));
        assert!(!cut.ends_with(' '));
    }

    #[test]
    fn a_preview_with_no_boundary_near_the_limit_is_cut_anyway_rather_than_returned_whole() {
        // One word longer than the limit, so there is nowhere good to cut and it is cut regardless.
        let unbroken = "x".repeat(400);
        assert_eq!(preview(&unbroken).chars().count(), 200);
    }

    #[test]
    fn a_preview_cuts_on_a_character_boundary_and_not_a_byte_one() {
        // Every character here is three bytes, so a byte index would panic or split one in half.
        let text = "\u{3042}".repeat(400);
        assert_eq!(preview(&text).chars().count(), 200);
    }
}
