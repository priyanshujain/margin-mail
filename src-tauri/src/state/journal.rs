// The write path, and the reason the tables are a view of the log rather than the log a record of
// the tables.
//
// Every change to the state database is an event appended here and then applied to the table it
// belongs to. Nothing writes a materialised table any other way, and `replay` rebuilds every one of
// them from an empty database. That property is the whole file. A row written directly is a fact
// this device holds and no other, and it disappears the moment another device replays its log over
// the top of it; a row written from an event is a fact any device can reproduce, which is what
// makes a backup worth restoring and a second device possible without a server.
//
// Merging is last writer wins per key by `at_ms`, with the device id breaking a tie. The tie break
// is not cosmetic: with it the winner is a function of the events rather than of the order they
// arrived in, so two devices that met the same pair in opposite orders land in the same place, and
// a replay may sort the log however it likes as long as the sort is total.
//
// A delete removes its row and leaves its event behind, and that event is the tombstone. Ownership
// of a key is therefore read from the row when there is one and from the log when there is not,
// because a delete that arrives before the creation it deletes still has to win.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::mirror::write::now_ms;

use super::device;

/// The kinds are the tables. An event of a kind this build does not know is kept and skipped rather
/// than refused: it was written by a newer build on the same account, it is not this device's to
/// throw away, and an upgrade replays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    SenderRule,
    Pile,
    Snooze,
    Note,
    Rename,
    Merge,
    Clip,
    ThreadFlag,
    Contact,
    Pref,
    Marker,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::SenderRule,
        Kind::Pile,
        Kind::Snooze,
        Kind::Note,
        Kind::Rename,
        Kind::Merge,
        Kind::Clip,
        Kind::ThreadFlag,
        Kind::Contact,
        Kind::Pref,
        Kind::Marker,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::SenderRule => "sender-rule",
            Kind::Pile => "pile",
            Kind::Snooze => "snooze",
            Kind::Note => "note",
            Kind::Rename => "rename",
            Kind::Merge => "merge",
            Kind::Clip => "clip",
            Kind::ThreadFlag => "thread-flag",
            Kind::Contact => "contact",
            Kind::Pref => "pref",
            Kind::Marker => "marker",
        }
    }

    pub fn parse(raw: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.as_str() == raw)
    }

    /// The table this kind materialises into and the column its key is that table's key in.
    fn table(self) -> (&'static str, &'static str) {
        match self {
            Kind::SenderRule => ("sender_rules", "subject"),
            Kind::Pile => ("piles", "thread_key"),
            Kind::Snooze => ("snoozes", "thread_key"),
            Kind::Note => ("notes", "id"),
            Kind::Rename => ("renames", "thread_key"),
            Kind::Merge => ("merges", "thread_key"),
            Kind::Clip => ("clips", "id"),
            Kind::ThreadFlag => ("thread_flags", "thread_key"),
            Kind::Contact => ("contacts", "address"),
            Kind::Pref => ("prefs", "key"),
            Kind::Marker => ("markers", "place"),
        }
    }
}

fn bundled() -> bool {
    true
}

/// What an event says about its key. Every payload is the whole value of the row rather than a
/// difference to it, so applying one never has to read what it replaces and the last writer is the
/// only one whose event has to be understood.
///
/// The absent case in each of them is the delete: no pile is a thread out of both piles, no return
/// is a snooze cancelled or spent, no merged key is an unmerge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Payload {
    /// Keyed on the address, or on the domain when `is_domain`.
    SenderRule {
        #[serde(default)]
        is_domain: bool,
        /// Absent is the delete, as it is in every other kind here: the sender goes back to having
        /// no rule at all, which is what puts them back in the Screener. It is optional rather than
        /// a fourth destination because "screened out" is a decision and "no rule" is the absence
        /// of one, and a Screener card undone has to reach the second.
        #[serde(default)]
        destination: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    Pile {
        #[serde(default)]
        pile: Option<String>,
        #[serde(default)]
        position: i64,
    },
    Snooze {
        #[serde(default)]
        return_at: Option<i64>,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        watermark: i64,
    },
    /// Keyed on the note id rather than on the thread, because two devices can write different
    /// notes on the same thread and both have to survive. Deletion is the flag rather than the
    /// absence of the row, for the same reason a tombstone exists at all.
    Note {
        thread_key: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        after_message_id: Option<String>,
        #[serde(default)]
        created_at: i64,
        #[serde(default)]
        deleted: bool,
    },
    Rename {
        #[serde(default)]
        name: Option<String>,
    },
    /// Keyed on the source thread key, pointing at the thread it now shows as part of.
    Merge {
        #[serde(default)]
        merged_key: Option<String>,
    },
    Clip {
        thread_key: String,
        #[serde(default)]
        message_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        sender_name: Option<String>,
        #[serde(default)]
        sender_address: String,
        #[serde(default)]
        subject: String,
        #[serde(default)]
        created_at: i64,
        #[serde(default)]
        deleted: bool,
    },
    ThreadFlag {
        #[serde(default)]
        ignored: bool,
        #[serde(default)]
        notify: bool,
    },
    Contact {
        #[serde(default)]
        note: Option<String>,
        #[serde(default)]
        notify: bool,
        #[serde(default)]
        allow_remote_images: bool,
        #[serde(default)]
        auto_trash_days: Option<i64>,
        #[serde(default = "bundled")]
        bundle: bool,
    },
    Pref {
        #[serde(default)]
        value: Option<String>,
    },
    Marker {
        #[serde(default)]
        seen_ms: i64,
    },
}

impl Payload {
    pub fn kind(&self) -> Kind {
        match self {
            Payload::SenderRule { .. } => Kind::SenderRule,
            Payload::Pile { .. } => Kind::Pile,
            Payload::Snooze { .. } => Kind::Snooze,
            Payload::Note { .. } => Kind::Note,
            Payload::Rename { .. } => Kind::Rename,
            Payload::Merge { .. } => Kind::Merge,
            Payload::Clip { .. } => Kind::Clip,
            Payload::ThreadFlag { .. } => Kind::ThreadFlag,
            Payload::Contact { .. } => Kind::Contact,
            Payload::Pref { .. } => Kind::Pref,
            Payload::Marker { .. } => Kind::Marker,
        }
    }
}

/// A row of the log exactly as it is stored. The kind stays text here rather than becoming a `Kind`
/// because a record written by a newer build has to survive being read, held and handed on by an
/// older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub device_id: String,
    pub seq: i64,
    pub at_ms: i64,
    pub kind: String,
    pub key: String,
    pub payload: String,
}

impl Record {
    /// The total order the merge is defined by, and the order a replay applies the log in.
    pub fn order(&self) -> (i64, &str, i64) {
        (self.at_ms, self.device_id.as_str(), self.seq)
    }
}

/// What became of one record. Nothing here is a failure: an event this build cannot read is
/// skipped so the rest of the log still lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    Applied,
    Older,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub applied: usize,
    pub skipped: usize,
    /// The kinds this build could not read, named once each, so a device meeting a newer one can
    /// say what it did not understand instead of passing over it silently.
    pub unknown: Vec<String>,
}

impl Report {
    pub(super) fn skip(&mut self, kind: &str) {
        self.skipped += 1;
        if Kind::parse(kind).is_none() && !self.unknown.iter().any(|held| held == kind) {
            self.unknown.push(kind.to_string());
        }
    }
}

/// A savepoint rather than a transaction, so appending inside a caller that already has one open
/// is not an error.
pub fn atomically<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    conn.execute_batch("SAVEPOINT state_event;")
        .map_err(|e| e.to_string())?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE state_event;")
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO state_event; RELEASE state_event;");
            Err(e)
        }
    }
}

pub fn append(conn: &Connection, key: &str, payload: &Payload) -> Result<Record, String> {
    append_at(conn, now_ms(), key, payload)
}

/// The same with the moment named, for a caller that already has one and for the tests that have to
/// put two devices in the same millisecond.
///
/// The event is kept whether or not it lands. A local change losing to one already held is a device
/// whose clock is behind another's, and the record of what this person did belongs in the log
/// either way.
pub fn append_at(
    conn: &Connection,
    at_ms: i64,
    key: &str,
    payload: &Payload,
) -> Result<Record, String> {
    let device_id = device::device_id(conn)?;
    let body = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    atomically(conn, |conn| {
        let record = Record {
            seq: next_seq(conn, &device_id)?,
            device_id: device_id.clone(),
            at_ms,
            kind: payload.kind().as_str().to_string(),
            key: key.to_string(),
            payload: body,
        };
        insert(conn, &record)?;
        apply(conn, &record)?;
        Ok(record)
    })
}

/// Puts a record in the log without applying it, and says whether it was new. An event from another
/// device is kept whether or not this build can read it: dropping one would lose it for the next
/// device that asks this one what it has.
pub(super) fn insert(conn: &Connection, record: &Record) -> Result<bool, String> {
    let written = conn
        .execute(
            "INSERT OR IGNORE INTO state.journal (device_id, seq, at_ms, kind, key, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.device_id,
                record.seq,
                record.at_ms,
                record.kind,
                record.key,
                record.payload
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(written > 0)
}

fn next_seq(conn: &Connection, device_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM state.journal WHERE device_id = ?1",
        [device_id],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Lands one event on its table if it is still the last word on its key.
pub fn apply(conn: &Connection, record: &Record) -> Result<Landing, String> {
    let Some(kind) = Kind::parse(&record.kind) else {
        return Ok(Landing::Skipped);
    };
    let Ok(payload) = serde_json::from_str::<Payload>(&record.payload) else {
        return Ok(Landing::Skipped);
    };
    if !wins(conn, kind, record)? {
        return Ok(Landing::Older);
    }
    materialise(conn, record, &payload)?;
    Ok(Landing::Applied)
}

/// Whether this event is still the last word on its key. Greater than or equal rather than greater
/// than, because an equal moment and an equal device means one device wrote twice inside a
/// millisecond and the event in hand is the second one.
fn wins(conn: &Connection, kind: Kind, record: &Record) -> Result<bool, String> {
    let held = match row_owner(conn, kind, &record.key)? {
        Some(owner) => Some(owner),
        None => log_owner(conn, record)?,
    };
    Ok(match held {
        None => true,
        Some((at_ms, device_id)) => {
            (record.at_ms, record.device_id.as_str()) >= (at_ms, device_id.as_str())
        }
    })
}

/// Who set the value a key currently holds. Not a bound parameter: a table name cannot be one, and
/// both halves come from the enum above rather than from anybody's input.
fn row_owner(conn: &Connection, kind: Kind, key: &str) -> Result<Option<(i64, String)>, String> {
    let (table, column) = kind.table();
    conn.query_row(
        &format!("SELECT at_ms, device_id FROM state.{table} WHERE {column} = ?1"),
        [key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Who owns a key whose row is gone. A delete leaves its mark in the log and nowhere else, so this
/// is what stops a creation that arrives after the deletion of it from coming back.
fn log_owner(conn: &Connection, record: &Record) -> Result<Option<(i64, String)>, String> {
    conn.query_row(
        "SELECT at_ms, device_id FROM state.journal
          WHERE kind = ?1 AND key = ?2 AND NOT (device_id = ?3 AND seq = ?4)
          ORDER BY at_ms DESC, device_id DESC, seq DESC
          LIMIT 1",
        params![record.kind, record.key, record.device_id, record.seq],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn materialise(conn: &Connection, record: &Record, payload: &Payload) -> Result<(), String> {
    let at = record.at_ms;
    let by = &record.device_id;
    let key = &record.key;
    match payload {
        Payload::SenderRule {
            is_domain,
            destination,
            reason,
        } => match destination {
            Some(destination) => conn.execute(
                "INSERT INTO state.sender_rules (subject, is_domain, destination, reason, at_ms, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(subject) DO UPDATE SET
                     is_domain = excluded.is_domain, destination = excluded.destination,
                     reason = excluded.reason, at_ms = excluded.at_ms, device_id = excluded.device_id",
                params![key, is_domain, destination, reason, at, by],
            ),
            None => conn.execute("DELETE FROM state.sender_rules WHERE subject = ?1", [key]),
        },
        Payload::Pile { pile, position } => match pile {
            Some(pile) => conn.execute(
                "INSERT INTO state.piles (thread_key, pile, position, at_ms, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(thread_key) DO UPDATE SET
                     pile = excluded.pile, position = excluded.position,
                     at_ms = excluded.at_ms, device_id = excluded.device_id",
                params![key, pile, position, at, by],
            ),
            None => conn.execute("DELETE FROM state.piles WHERE thread_key = ?1", [key]),
        },
        Payload::Snooze {
            return_at,
            kind,
            watermark,
        } => match return_at {
            Some(return_at) => conn.execute(
                "INSERT INTO state.snoozes (thread_key, return_at, kind, watermark, at_ms, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(thread_key) DO UPDATE SET
                     return_at = excluded.return_at, kind = excluded.kind,
                     watermark = excluded.watermark, at_ms = excluded.at_ms,
                     device_id = excluded.device_id",
                params![key, return_at, kind, watermark, at, by],
            ),
            None => conn.execute("DELETE FROM state.snoozes WHERE thread_key = ?1", [key]),
        },
        Payload::Note {
            thread_key,
            body,
            after_message_id,
            created_at,
            deleted,
        } => conn.execute(
            "INSERT INTO state.notes
                 (id, thread_key, body, after_message_id, created_at, at_ms, device_id, deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 thread_key = excluded.thread_key, body = excluded.body,
                 after_message_id = excluded.after_message_id, created_at = excluded.created_at,
                 at_ms = excluded.at_ms, device_id = excluded.device_id,
                 deleted = excluded.deleted",
            params![key, thread_key, body, after_message_id, created_at, at, by, deleted],
        ),
        Payload::Rename { name } => match name {
            Some(name) => conn.execute(
                "INSERT INTO state.renames (thread_key, name, at_ms, device_id)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(thread_key) DO UPDATE SET
                     name = excluded.name, at_ms = excluded.at_ms, device_id = excluded.device_id",
                params![key, name, at, by],
            ),
            None => conn.execute("DELETE FROM state.renames WHERE thread_key = ?1", [key]),
        },
        Payload::Merge { merged_key } => match merged_key {
            Some(merged_key) => conn.execute(
                "INSERT INTO state.merges (thread_key, merged_key, at_ms, device_id)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(thread_key) DO UPDATE SET
                     merged_key = excluded.merged_key, at_ms = excluded.at_ms,
                     device_id = excluded.device_id",
                params![key, merged_key, at, by],
            ),
            None => conn.execute("DELETE FROM state.merges WHERE thread_key = ?1", [key]),
        },
        Payload::Clip {
            thread_key,
            message_id,
            text,
            sender_name,
            sender_address,
            subject,
            created_at,
            deleted,
        } => conn.execute(
            "INSERT INTO state.clips
                 (id, thread_key, message_id, text, sender_name, sender_address, subject,
                  created_at, at_ms, device_id, deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 thread_key = excluded.thread_key, message_id = excluded.message_id,
                 text = excluded.text, sender_name = excluded.sender_name,
                 sender_address = excluded.sender_address, subject = excluded.subject,
                 created_at = excluded.created_at, at_ms = excluded.at_ms,
                 device_id = excluded.device_id, deleted = excluded.deleted",
            params![
                key,
                thread_key,
                message_id,
                text,
                sender_name,
                sender_address,
                subject,
                created_at,
                at,
                by,
                deleted
            ],
        ),
        Payload::ThreadFlag { ignored, notify } => conn.execute(
            "INSERT INTO state.thread_flags (thread_key, ignored, notify, at_ms, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(thread_key) DO UPDATE SET
                 ignored = excluded.ignored, notify = excluded.notify,
                 at_ms = excluded.at_ms, device_id = excluded.device_id",
            params![key, ignored, notify, at, by],
        ),
        Payload::Contact {
            note,
            notify,
            allow_remote_images,
            auto_trash_days,
            bundle,
        } => conn.execute(
            "INSERT INTO state.contacts
                 (address, note, notify, allow_remote_images, auto_trash_days, bundle,
                  at_ms, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(address) DO UPDATE SET
                 note = excluded.note, notify = excluded.notify,
                 allow_remote_images = excluded.allow_remote_images,
                 auto_trash_days = excluded.auto_trash_days, bundle = excluded.bundle,
                 at_ms = excluded.at_ms, device_id = excluded.device_id",
            params![
                key,
                note,
                notify,
                allow_remote_images,
                auto_trash_days,
                bundle,
                at,
                by
            ],
        ),
        Payload::Pref { value } => match value {
            Some(value) => conn.execute(
                "INSERT INTO state.prefs (key, value, at_ms, device_id) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET
                     value = excluded.value, at_ms = excluded.at_ms,
                     device_id = excluded.device_id",
                params![key, value, at, by],
            ),
            None => conn.execute("DELETE FROM state.prefs WHERE key = ?1", [key]),
        },
        Payload::Marker { seen_ms } => conn.execute(
            "INSERT INTO state.markers (place, seen_ms, at_ms, device_id) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(place) DO UPDATE SET
                 seen_ms = excluded.seen_ms, at_ms = excluded.at_ms,
                 device_id = excluded.device_id",
            params![key, seen_ms, at, by],
        ),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The whole log, in the order a replay applies it.
pub fn records(conn: &Connection) -> Result<Vec<Record>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT device_id, seq, at_ms, kind, key, payload FROM state.journal
              ORDER BY at_ms ASC, device_id ASC, seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Record {
                device_id: row.get(0)?,
                seq: row.get(1)?,
                at_ms: row.get(2)?,
                kind: row.get(3)?,
                key: row.get(4)?,
                payload: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Rebuilds every table from the log. The one function allowed to write these tables from anything
/// but a fresh event, and the proof that they are a view of it: run this over an empty database and
/// what comes back is what was there.
///
/// It applies in order without asking who owns a key, because the order is the same total order the
/// tie break defines, so the last event for a key is the last one applied. Resolution is for events
/// arriving one at a time; a rebuild already knows the end of the story.
pub fn replay(conn: &Connection) -> Result<Report, String> {
    atomically(conn, |conn| {
        for kind in Kind::ALL {
            let (table, _) = kind.table();
            conn.execute(&format!("DELETE FROM state.{table}"), [])
                .map_err(|e| e.to_string())?;
        }
        let mut report = Report::default();
        for record in records(conn)? {
            let payload = Kind::parse(&record.kind)
                .and_then(|_| serde_json::from_str::<Payload>(&record.payload).ok());
            match payload {
                Some(payload) => {
                    materialise(conn, &record, &payload)?;
                    report.applied += 1;
                }
                None => report.skip(&record.kind),
            }
        }
        Ok(report)
    })
}
