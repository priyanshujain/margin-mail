// Everything that changes a decision. Every function here is one event appended to the log and then
// applied by it, so none of them writes a table: the table is downstream of the event, and
// `journal` is the only place that knows how one becomes the other. The SQL that is here reads
// what a patch needs to complete itself, and writes the one table that is outside the log.
//
// The functions that take a patch rather than a value read the current row first and append the
// whole of what it becomes. That is deliberate. A payload that carried only the difference would
// have to be applied to the row it was written against, and on a second device that row may be
// something else or may not exist at all; a payload that carries the whole value is a fact about a
// key at a moment, and last writer wins can decide between two of them without either one having to
// be replayed in a particular order.

use rusqlite::{Connection, OptionalExtension};

use crate::dto::{ContactPatch, Destination, Person, Pile, SnoozeKind};
use crate::mirror::write::{fresh_id, now_ms};

use super::journal::{append, atomically, Payload};
use super::read;

/// Domains where "everyone at this domain" is not a group of any kind. A domain rule on one of
/// these would route a third of the world's mail with one keystroke, so the rule is refused rather
/// than offered and quietly regretted.
pub const CONSUMER_DOMAINS: [&str; 22] = [
    "aol.com",
    "fastmail.com",
    "gmail.com",
    "gmx.com",
    "gmx.de",
    "googlemail.com",
    "hey.com",
    "hotmail.co.uk",
    "hotmail.com",
    "icloud.com",
    "live.com",
    "mac.com",
    "mail.com",
    "me.com",
    "msn.com",
    "outlook.com",
    "pm.me",
    "proton.me",
    "protonmail.com",
    "yahoo.co.uk",
    "yahoo.com",
    "yandex.ru",
];

pub fn is_consumer_domain(domain: &str) -> bool {
    CONSUMER_DOMAINS.contains(&domain.trim().to_lowercase().as_str())
}

/// The part of an address after the at sign, lowercased, which is what a domain rule is keyed on.
pub fn domain_of(address: &str) -> Option<String> {
    let lowered = address.trim().to_lowercase();
    let (_, domain) = lowered.rsplit_once('@')?;
    (!domain.is_empty()).then(|| domain.to_string())
}

pub fn destination_name(destination: Destination) -> &'static str {
    match destination {
        Destination::Inbox => "inbox",
        Destination::Feed => "feed",
        Destination::PaperTrail => "paper-trail",
        Destination::ScreenedOut => "screened-out",
    }
}

pub fn pile_name(pile: Pile) -> &'static str {
    match pile {
        Pile::ReplyLater => "reply-later",
        Pile::SetAside => "set-aside",
    }
}

pub fn snooze_name(kind: SnoozeKind) -> &'static str {
    match kind {
        SnoozeKind::LaterToday => "later-today",
        SnoozeKind::Tomorrow => "tomorrow",
        SnoozeKind::Weekend => "weekend",
        SnoozeKind::NextWeek => "next-week",
        SnoozeKind::Date => "date",
        SnoozeKind::IfNoReply => "if-no-reply",
    }
}

// ---------------------------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------------------------

/// Decides a sender, by address or by their whole domain. There is no unset: every path through the
/// Screener and the contact card names a destination, and a sender with no rule is a sender who has
/// never been decided rather than one who has been undecided.
pub fn set_rule(
    conn: &Connection,
    subject: &str,
    is_domain: bool,
    destination: Destination,
    reason: Option<&str>,
) -> Result<(), String> {
    let subject = subject.trim().to_lowercase();
    if subject.is_empty() {
        return Err("a rule needs an address or a domain to be about".into());
    }
    if is_domain && is_consumer_domain(&subject) {
        return Err(format!(
            "everyone at {subject} is not one sender, so a rule cannot be set for the whole domain"
        ));
    }
    append(
        conn,
        &subject,
        &Payload::SenderRule {
            is_domain,
            destination: Some(destination_name(destination).to_string()),
            reason: reason.map(|reason| reason.to_string()),
        },
    )
    .map(|_| ())
}

/// Takes a sender's rule away, so they are somebody with no decision about them again and their
/// next message waits in the Screener.
///
/// This is what undoing a Screener card does, and it is the reason the sender rule payload has an
/// absent case. It is not the same as screening somebody out: that is a decision, and this is the
/// absence of one.
pub fn clear_rule(conn: &Connection, address: &str) -> Result<(), String> {
    let subject = address.trim().to_lowercase();
    append(
        conn,
        &subject,
        &Payload::SenderRule {
            is_domain: false,
            destination: None,
            reason: None,
        },
    )
    .map(|_| ())?;
    // A domain rule can be what was deciding them, and taking the address rule away would leave it
    // in force. Undoing a decision has to leave the sender genuinely undecided.
    if let Some(domain) = domain_of(&subject) {
        let has_domain_rule: bool = conn
            .query_row(
                "SELECT 1 FROM state.sender_rules WHERE is_domain = 1 AND subject = ?1",
                [&domain],
                |_| Ok(true),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(false);
        if has_domain_rule {
            append(
                conn,
                &domain,
                &Payload::SenderRule {
                    is_domain: true,
                    destination: None,
                    reason: None,
                },
            )
            .map(|_| ())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Piles
// ---------------------------------------------------------------------------------------------

/// Puts a thread on top of a pile. A pile is a stack, so its order is part of the state and the top
/// of it is the last thing put there.
pub fn set_pile(conn: &Connection, thread_key: &str, pile: Pile) -> Result<(), String> {
    let top: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM state.piles WHERE pile = ?1",
            [pile_name(pile)],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    set_pile_at(conn, thread_key, pile, top)
}

/// The same with the place in the stack named, which is what reordering one is.
pub fn set_pile_at(
    conn: &Connection,
    thread_key: &str,
    pile: Pile,
    position: i64,
) -> Result<(), String> {
    append(
        conn,
        thread_key,
        &Payload::Pile {
            pile: Some(pile_name(pile).to_string()),
            position,
        },
    )
    .map(|_| ())
}

pub fn clear_pile(conn: &Connection, thread_key: &str) -> Result<(), String> {
    append(
        conn,
        thread_key,
        &Payload::Pile {
            pile: None,
            position: 0,
        },
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Snoozes
// ---------------------------------------------------------------------------------------------

pub fn set_snooze(
    conn: &Connection,
    thread_key: &str,
    return_at: i64,
    kind: SnoozeKind,
    watermark: i64,
) -> Result<(), String> {
    append(
        conn,
        thread_key,
        &Payload::Snooze {
            return_at: Some(return_at),
            kind: snooze_name(kind).to_string(),
            watermark,
        },
    )
    .map(|_| ())
}

/// Cancelled by hand, or spent because the thread came back.
pub fn clear_snooze(conn: &Connection, thread_key: &str) -> Result<(), String> {
    append(
        conn,
        thread_key,
        &Payload::Snooze {
            return_at: None,
            kind: String::new(),
            watermark: 0,
        },
    )
    .map(|_| ())
}

/// A snooze that has come back and is waiting in the Back group. Outside the log on purpose: the
/// table has neither a device id nor a tombstone, so it cannot take part in the merge, and what it
/// records is this device's screen rather than the person's decision. It lives in the state file
/// only because the mirror can be rebuilt and a thread that came back must not come back twice.
pub fn mark_returned(conn: &Connection, thread_key: &str, due_ms: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO state.returned (thread_key, at_ms, due_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(thread_key) DO UPDATE SET at_ms = excluded.at_ms, due_ms = excluded.due_ms",
        rusqlite::params![thread_key, now_ms(), due_ms],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn clear_returned(conn: &Connection, thread_key: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM state.returned WHERE thread_key = ?1",
        [thread_key],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------------------------

pub fn add_note(
    conn: &Connection,
    thread_key: &str,
    body: &str,
    after_message_id: Option<&str>,
) -> Result<String, String> {
    let id = fresh_id("note");
    append(
        conn,
        &id,
        &Payload::Note {
            thread_key: thread_key.to_string(),
            body: body.to_string(),
            after_message_id: after_message_id.map(|id| id.to_string()),
            created_at: now_ms(),
            deleted: false,
        },
    )?;
    Ok(id)
}

pub fn edit_note(conn: &Connection, id: &str, body: &str) -> Result<(), String> {
    let held = note_payload(conn, id)?;
    let Payload::Note {
        thread_key,
        after_message_id,
        created_at,
        deleted,
        ..
    } = held
    else {
        return Err("that note is not here".into());
    };
    append(
        conn,
        id,
        &Payload::Note {
            thread_key,
            body: body.to_string(),
            after_message_id,
            created_at,
            deleted,
        },
    )
    .map(|_| ())
}

/// A flag rather than a removed row, because a note deleted on one device and edited on another has
/// to resolve the same way whichever order the two events arrive in.
pub fn delete_note(conn: &Connection, id: &str) -> Result<(), String> {
    let held = note_payload(conn, id)?;
    let Payload::Note {
        thread_key,
        body,
        after_message_id,
        created_at,
        ..
    } = held
    else {
        return Err("that note is not here".into());
    };
    append(
        conn,
        id,
        &Payload::Note {
            thread_key,
            body,
            after_message_id,
            created_at,
            deleted: true,
        },
    )
    .map(|_| ())
}

fn note_payload(conn: &Connection, id: &str) -> Result<Payload, String> {
    conn.query_row(
        "SELECT thread_key, body, after_message_id, created_at, deleted FROM state.notes
          WHERE id = ?1",
        [id],
        |row| {
            Ok(Payload::Note {
                thread_key: row.get(0)?,
                body: row.get(1)?,
                after_message_id: row.get(2)?,
                created_at: row.get(3)?,
                deleted: row.get(4)?,
            })
        },
    )
    .map_err(|_| "that note is not here".to_string())
}

// ---------------------------------------------------------------------------------------------
// Renames and merges
// ---------------------------------------------------------------------------------------------

pub fn set_rename(conn: &Connection, thread_key: &str, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return clear_rename(conn, thread_key);
    }
    append(
        conn,
        thread_key,
        &Payload::Rename {
            name: Some(name.to_string()),
        },
    )
    .map(|_| ())
}

pub fn clear_rename(conn: &Connection, thread_key: &str) -> Result<(), String> {
    append(conn, thread_key, &Payload::Rename { name: None }).map(|_| ())
}

/// Makes several threads show as one. Each source gets its own event, because each is its own key
/// and two devices merging overlapping sets have to be able to meet without either being wrong.
pub fn merge_threads(
    conn: &Connection,
    sources: &[String],
    merged_key: &str,
) -> Result<(), String> {
    if merged_key.trim().is_empty() {
        return Err("a merge needs a thread to merge into".into());
    }
    atomically(conn, |conn| {
        for source in sources {
            if source == merged_key {
                continue;
            }
            append(
                conn,
                source,
                &Payload::Merge {
                    merged_key: Some(merged_key.to_string()),
                },
            )?;
        }
        Ok(())
    })
}

/// Breaks a merged thread back into the threads it was made of. Only the sources that point
/// directly at it: a merge of a merge is undone one layer at a time, which is the layer the banner
/// on that thread is offering to undo.
pub fn unmerge(conn: &Connection, merged_key: &str) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT thread_key FROM state.merges WHERE merged_key = ?1")
        .map_err(|e| e.to_string())?;
    let sources = stmt
        .query_map([merged_key], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    atomically(conn, |conn| {
        for source in sources {
            append(conn, &source, &Payload::Merge { merged_key: None })?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------------------------

pub fn add_clip(
    conn: &Connection,
    thread_key: &str,
    message_id: &str,
    text: &str,
    sender: &Person,
    subject: &str,
) -> Result<String, String> {
    let id = fresh_id("clip");
    append(
        conn,
        &id,
        &Payload::Clip {
            thread_key: thread_key.to_string(),
            message_id: message_id.to_string(),
            text: text.to_string(),
            sender_name: sender.name.clone(),
            sender_address: sender.address.to_lowercase(),
            subject: subject.to_string(),
            created_at: now_ms(),
            deleted: false,
        },
    )?;
    Ok(id)
}

pub fn delete_clip(conn: &Connection, id: &str) -> Result<(), String> {
    let held = conn
        .query_row(
            "SELECT thread_key, message_id, text, sender_name, sender_address, subject, created_at
               FROM state.clips WHERE id = ?1",
            [id],
            |row| {
                Ok(Payload::Clip {
                    thread_key: row.get(0)?,
                    message_id: row.get(1)?,
                    text: row.get(2)?,
                    sender_name: row.get(3)?,
                    sender_address: row.get(4)?,
                    subject: row.get(5)?,
                    created_at: row.get(6)?,
                    deleted: true,
                })
            },
        )
        .map_err(|_| "that clip is not here".to_string())?;
    append(conn, id, &held).map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Threads and people
// ---------------------------------------------------------------------------------------------

/// Ignore and notify are one row because they are set on the same thing by the same gesture, so a
/// change to either writes the value of both.
pub fn set_thread_flags(
    conn: &Connection,
    thread_key: &str,
    ignored: Option<bool>,
    notify: Option<bool>,
) -> Result<(), String> {
    let held = read::flags_of(conn, thread_key)?;
    append(
        conn,
        thread_key,
        &Payload::ThreadFlag {
            ignored: ignored.unwrap_or(held.ignored),
            notify: notify.unwrap_or(held.notify),
        },
    )
    .map(|_| ())
}

/// The contact card's fields. The destination and the domain toggle on the same card are the sender
/// rule and go through `set_rule`, because a rule can be about a domain and a contact never is.
pub fn set_contact(conn: &Connection, address: &str, patch: &ContactPatch) -> Result<(), String> {
    let address = address.trim().to_lowercase();
    if address.is_empty() {
        return Err("a contact needs an address".into());
    }
    let held = read::contact(conn, &address)?.unwrap_or_else(|| read::Contact::unknown(&address));
    append(
        conn,
        &address,
        &Payload::Contact {
            note: patch.note.clone().or(held.note),
            notify: patch.notify.unwrap_or(held.notify),
            allow_remote_images: patch.allow_remote_images.unwrap_or(held.allow_remote_images),
            auto_trash_days: patch
                .auto_trash_days
                .unwrap_or(held.auto_trash_days)
                .map(|days| days as i64),
            bundle: patch.bundle.unwrap_or(held.bundle),
        },
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Preferences and markers
// ---------------------------------------------------------------------------------------------

/// The preferences that follow the person rather than the machine: the signature, the instant intro
/// text. Anything about this device belongs in settings.json instead.
pub fn set_pref(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    append(
        conn,
        key,
        &Payload::Pref {
            value: Some(value.to_string()),
        },
    )
    .map(|_| ())
}

pub fn clear_pref(conn: &Connection, key: &str) -> Result<(), String> {
    append(conn, key, &Payload::Pref { value: None }).map(|_| ())
}

pub fn set_marker(conn: &Connection, place: &str, seen_ms: i64) -> Result<(), String> {
    append(conn, place, &Payload::Marker { seen_ms }).map(|_| ())
}
