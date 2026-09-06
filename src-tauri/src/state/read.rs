// What the rest of the app is allowed to know about the decisions, so that nothing outside this
// module writes SQL against these tables.
//
// The one thing here that is not a lookup is `effective_key`. A merge maps a source thread key onto
// the key of the thread it now shows as part of; merging a merged thread makes a chain; and a view
// that does not walk the chain shows the same conversation twice. Every view goes through this
// function, or through `CHAIN` when it needs the same answer inside one statement across both
// databases.

use rusqlite::{Connection, OptionalExtension};

use crate::dto::{Clip, Destination, Note, Person, Pile, SenderRule, Snooze, SnoozeKind};

use super::write::domain_of;

pub fn destination_named(raw: &str) -> Option<Destination> {
    Some(match raw {
        "inbox" => Destination::Inbox,
        "feed" => Destination::Feed,
        "paper-trail" => Destination::PaperTrail,
        "screened-out" => Destination::ScreenedOut,
        _ => return None,
    })
}

pub fn pile_named(raw: &str) -> Option<Pile> {
    Some(match raw {
        "reply-later" => Pile::ReplyLater,
        "set-aside" => Pile::SetAside,
        _ => return None,
    })
}

pub fn snooze_named(raw: &str) -> Option<SnoozeKind> {
    Some(match raw {
        "later-today" => SnoozeKind::LaterToday,
        "tomorrow" => SnoozeKind::Tomorrow,
        "weekend" => SnoozeKind::Weekend,
        "next-week" => SnoozeKind::NextWeek,
        "date" => SnoozeKind::Date,
        "if-no-reply" => SnoozeKind::IfNoReply,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------------------------

/// The rule that decides a sender. An address rule beats a domain rule, which is an order in one
/// query rather than two tables and two lookups.
///
/// A destination this build does not know is no rule at all rather than an error: the sender waits
/// in the Screener until the build catches up, which is recoverable, and a read that refuses is
/// not.
pub fn rule_for(
    conn: &Connection,
    account_id: &str,
    address: &str,
) -> Result<Option<SenderRule>, String> {
    let address = address.trim().to_lowercase();
    let domain = domain_of(&address).unwrap_or_default();
    conn.query_row(
        "SELECT subject, is_domain, destination, reason, at_ms FROM state.sender_rules
          WHERE (is_domain = 0 AND subject = ?1) OR (is_domain = 1 AND subject = ?2)
          ORDER BY is_domain ASC
          LIMIT 1",
        [&address, &domain],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        },
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|found| {
        found.and_then(|(subject, is_domain, destination, reason, at_ms)| {
            Some(SenderRule {
                account_id: account_id.to_string(),
                subject,
                is_domain,
                destination: destination_named(&destination)?,
                decided_at_ms: at_ms,
                reason,
            })
        })
    })
}

/// The same question when only the answer matters, which is once a message during routing.
pub fn destination_for(conn: &Connection, address: &str) -> Result<Option<Destination>, String> {
    let address = address.trim().to_lowercase();
    let domain = domain_of(&address).unwrap_or_default();
    let found: Option<String> = conn
        .query_row(
            "SELECT destination FROM state.sender_rules
              WHERE (is_domain = 0 AND subject = ?1) OR (is_domain = 1 AND subject = ?2)
              ORDER BY is_domain ASC
              LIMIT 1",
            [&address, &domain],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(found.as_deref().and_then(destination_named))
}

/// Every rule, for the Contacts place.
pub fn rules(conn: &Connection, account_id: &str) -> Result<Vec<SenderRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT subject, is_domain, destination, reason, at_ms FROM state.sender_rules
              ORDER BY subject ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (subject, is_domain, destination, reason, at_ms) = row.map_err(|e| e.to_string())?;
        let Some(destination) = destination_named(&destination) else {
            continue;
        };
        out.push(SenderRule {
            account_id: account_id.to_string(),
            subject,
            is_domain,
            destination,
            decided_at_ms: at_ms,
            reason,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Piles and snoozes
// ---------------------------------------------------------------------------------------------

pub fn pile_of(conn: &Connection, thread_key: &str) -> Result<Option<Pile>, String> {
    let held: Option<String> = conn
        .query_row(
            "SELECT pile FROM state.piles WHERE thread_key = ?1",
            [thread_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(held.as_deref().and_then(pile_named))
}

/// A pile is a stack of cards, so the last thread put on it is the one on top.
pub fn pile_keys(conn: &Connection, pile: Pile) -> Result<Vec<String>, String> {
    let name = super::write::pile_name(pile);
    let mut stmt = conn
        .prepare(
            "SELECT thread_key FROM state.piles WHERE pile = ?1
              ORDER BY position DESC, thread_key ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([name], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// A snooze as the state database holds it. `dto::Snooze` is this without the watermark, which is
/// the evaluator's business and never the frontend's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeRow {
    pub thread_key: String,
    pub return_at: i64,
    pub kind: SnoozeKind,
    /// For `if-no-reply`: the latest message when the snooze was set, so a reply since is visible.
    pub watermark: i64,
}

impl SnoozeRow {
    pub fn view(&self) -> Snooze {
        Snooze {
            thread_key: self.thread_key.clone(),
            return_at_ms: self.return_at,
            kind: self.kind,
        }
    }
}

fn snooze_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<SnoozeRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (thread_key, return_at, kind, watermark) = row.map_err(|e| e.to_string())?;
        let Some(kind) = snooze_named(&kind) else {
            continue;
        };
        out.push(SnoozeRow {
            thread_key,
            return_at,
            kind,
            watermark,
        });
    }
    Ok(out)
}

const SNOOZE_COLUMNS: &str = "SELECT thread_key, return_at, kind, watermark FROM state.snoozes";

pub fn snooze_of(conn: &Connection, thread_key: &str) -> Result<Option<SnoozeRow>, String> {
    let found = snooze_rows(
        conn,
        &format!("{SNOOZE_COLUMNS} WHERE thread_key = ?1"),
        [thread_key],
    )?;
    Ok(found.into_iter().next())
}

/// Every snooze whose moment has passed. Nothing runs in the background, so this is what waking up
/// asks, and a thread that is late says so rather than pretending it returned on time.
pub fn due_snoozes(conn: &Connection, now: i64) -> Result<Vec<SnoozeRow>, String> {
    snooze_rows(
        conn,
        &format!("{SNOOZE_COLUMNS} WHERE return_at <= ?1 ORDER BY return_at ASC"),
        [now],
    )
}

/// Everything waiting to return, soonest first, for the Snoozed place.
pub fn snoozes(conn: &Connection) -> Result<Vec<SnoozeRow>, String> {
    snooze_rows(conn, &format!("{SNOOZE_COLUMNS} ORDER BY return_at ASC"), [])
}

/// Threads a snooze has brought back and that have not been opened since.
pub fn returned(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT thread_key FROM state.returned ORDER BY due_ms DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Notes, renames, merges, clips
// ---------------------------------------------------------------------------------------------

pub fn notes_on(conn: &Connection, thread_key: &str) -> Result<Vec<Note>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_key, body, created_at, after_message_id FROM state.notes
              WHERE thread_key = ?1 AND deleted = 0
              ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([thread_key], |row| {
            Ok(Note {
                id: row.get(0)?,
                thread_key: row.get(1)?,
                body: row.get(2)?,
                created_at_ms: row.get(3)?,
                after_message_id: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn rename_of(conn: &Connection, thread_key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT name FROM state.renames WHERE thread_key = ?1",
        [thread_key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// The walk from a source thread key to the thread it ends up in. `UNION` rather than `UNION ALL`,
/// so a cycle that should not exist stops rather than hangs, and the terminal is the key that is
/// not itself merged into something else.
///
/// Public because a view is one statement across both databases: a list that needs the effective
/// key puts this in front of its own select rather than asking Rust per row.
pub const CHAIN: &str = "\
    WITH RECURSIVE chain(source, key) AS (
        SELECT thread_key, merged_key FROM state.merges
        UNION
        SELECT c.source, m.merged_key FROM chain c JOIN state.merges m ON m.thread_key = c.key
    )";

/// The key a thread shows under. Its own, unless it has been merged into another, in which case the
/// end of that chain.
pub fn effective_key(conn: &Connection, thread_key: &str) -> Result<String, String> {
    let found: Option<String> = conn
        .query_row(
            &format!(
                "{CHAIN} SELECT key FROM chain
                  WHERE source = ?1 AND key NOT IN (SELECT thread_key FROM state.merges)
                  LIMIT 1"
            ),
            [thread_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(found.unwrap_or_else(|| thread_key.to_string()))
}

/// Every thread key that shows under this one, the ones merged into what was merged included.
pub fn merge_sources(conn: &Connection, merged_key: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "{CHAIN} SELECT DISTINCT source FROM chain WHERE key = ?1 ORDER BY source ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([merged_key], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn clips(conn: &Connection, account_id: &str, limit: u32) -> Result<Vec<Clip>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_key, message_id, text, sender_name, sender_address, subject,
                    created_at
               FROM state.clips WHERE deleted = 0
              ORDER BY created_at DESC, id DESC
              LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit], |row| {
            Ok(Clip {
                id: row.get(0)?,
                account_id: account_id.to_string(),
                thread_key: row.get(1)?,
                message_id: row.get(2)?,
                text: row.get(3)?,
                sender: Person {
                    name: row.get(4)?,
                    address: row.get(5)?,
                },
                subject: row.get(6)?,
                created_at_ms: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Threads and people
// ---------------------------------------------------------------------------------------------

/// The two per-thread switches. Absent is both off, which is why this is not an Option.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadFlags {
    pub ignored: bool,
    pub notify: bool,
}

pub fn flags_of(conn: &Connection, thread_key: &str) -> Result<ThreadFlags, String> {
    let held: Option<(bool, bool)> = conn
        .query_row(
            "SELECT ignored, notify FROM state.thread_flags WHERE thread_key = ?1",
            [thread_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(held
        .map(|(ignored, notify)| ThreadFlags { ignored, notify })
        .unwrap_or_default())
}

/// What the state database holds about one correspondent. The contact card is this plus their
/// recent threads and their files, which are the mirror's to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub address: String,
    pub note: Option<String>,
    pub notify: bool,
    pub allow_remote_images: bool,
    /// Trash this sender's Feed mail after so many days. None is never.
    pub auto_trash_days: Option<u32>,
    pub bundle: bool,
}

impl Contact {
    /// Somebody with no record yet, which is most people: the defaults are what the card shows
    /// before anything has been decided about them.
    pub fn unknown(address: &str) -> Contact {
        Contact {
            address: address.trim().to_lowercase(),
            note: None,
            notify: false,
            allow_remote_images: false,
            auto_trash_days: None,
            bundle: true,
        }
    }
}

pub fn contact(conn: &Connection, address: &str) -> Result<Option<Contact>, String> {
    let address = address.trim().to_lowercase();
    conn.query_row(
        "SELECT address, note, notify, allow_remote_images, auto_trash_days, bundle
           FROM state.contacts WHERE address = ?1",
        [&address],
        |row| {
            Ok(Contact {
                address: row.get(0)?,
                note: row.get(1)?,
                notify: row.get(2)?,
                allow_remote_images: row.get(3)?,
                auto_trash_days: row.get::<_, Option<i64>>(4)?.map(|days| days as u32),
                bundle: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Everyone with a record, for the Contacts place.
pub fn contacts(conn: &Connection) -> Result<Vec<Contact>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT address, note, notify, allow_remote_images, auto_trash_days, bundle
               FROM state.contacts ORDER BY address ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Contact {
                address: row.get(0)?,
                note: row.get(1)?,
                notify: row.get(2)?,
                allow_remote_images: row.get(3)?,
                auto_trash_days: row.get::<_, Option<i64>>(4)?.map(|days| days as u32),
                bundle: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Preferences and markers
// ---------------------------------------------------------------------------------------------

pub fn pref(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM state.prefs WHERE key = ?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn prefs(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM state.prefs ORDER BY key ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Where the "You left off here" line goes in a place.
pub fn marker(conn: &Connection, place: &str) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT seen_ms FROM state.markers WHERE place = ?1",
        [place],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}
