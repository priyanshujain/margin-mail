// Optimistic writes.
//
// A flag change lands in the mirror, renders, and is pushed behind. Consecutive changes to the
// same messages coalesce into one row, so starring and unstarring while the network is away
// reaches the provider once rather than twice, and a failure comes back with the reason on the row
// so the account chip can say what went wrong instead of spinning.
//
// A send is an outbox row too, with `hold_until` carrying its undo delay. It is the one operation
// here that is not safe to do twice, so it keeps its own bookkeeping in `crate::send`: the bytes
// are frozen when the row is queued, the row is leased before the call, and a row that has been
// attempted before asks the mirror whether the message already went before it tries again.
//
// A failure is one of two things. The connection, the token, a permission or the account's minute
// is not this row's fault and will pass or be fixed, so the row waits and the rows behind it wait
// with it. The provider refusing this row itself (a 4xx: an id it no longer has, a label it never
// had) will not pass, so the row is given one more try and then dropped with its reason recorded,
// because a queue that retries a dead row for ever is a queue that never moves and reports the
// same error on every pass for ever. That was the shape of the bug this file used to have.

use serde::{Deserialize, Serialize};

use crate::dto::FlagPatch;
use crate::mirror::write::{self, OutboxRow};
use crate::provider::ProviderError;

use super::{Outcome, Remote, Store};

/// How long a pass gives the queue before it moves on to the pull. A deeper queue drains further
/// on the next tick rather than holding the pull up.
pub const PUSH_BUDGET_MS: i64 = 20_000;
/// Quit is racing a timeout on the frontend, so the flush takes what it can get and returns.
pub const FLUSH_BUDGET_MS: i64 = 4_000;

const MAX_BACKOFF_MS: i64 = 60_000;

/// How many times a flag or label change is offered before a refusal is believed. Two: once to
/// hit a fluke, once to be sure. A 400 for an id that no longer exists does not get better with
/// waiting.
pub const MAX_WRITE_ATTEMPTS: i64 = 2;

/// What a drain did: the half-pass outcome the engine already reads, and the writes it gave up on,
/// each as a sentence for the person who made them.
#[derive(Debug, Default)]
pub struct Drained {
    pub outcome: Outcome,
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlagsPayload {
    pub ids: Vec<String>,
    pub patch: FlagPatch,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelsPayload {
    pub ids: Vec<String>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

/// Later wins per field, which is what a person clicking twice means.
fn merge(base: &FlagPatch, next: &FlagPatch) -> FlagPatch {
    FlagPatch {
        seen: next.seen.or(base.seen),
        starred: next.starred.or(base.starred),
        archived: next.archived.or(base.archived),
        trashed: next.trashed.or(base.trashed),
        spam: next.spam.or(base.spam),
    }
}

/// Queues a flag change, folding it into the row already waiting when that row covers exactly the
/// same messages and has not been attempted yet.
pub fn queue_flags(
    conn: &rusqlite::Connection,
    ids: &[String],
    patch: &FlagPatch,
    thread_key: Option<&str>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = write::now_ms();
    let mut sorted = ids.to_vec();
    sorted.sort();

    if let Some(row) = write::coalescable(conn, write::OP_FLAGS, now)? {
        if let Ok(held) = serde_json::from_str::<FlagsPayload>(&row.payload) {
            let mut theirs = held.ids.clone();
            theirs.sort();
            if theirs == sorted {
                let folded = FlagsPayload {
                    ids: held.ids,
                    patch: merge(&held.patch, patch),
                };
                return write::enqueue(
                    conn,
                    &OutboxRow {
                        payload: serde_json::to_string(&folded).map_err(|e| e.to_string())?,
                        ..row
                    },
                )
                .map(|_| ());
            }
        }
    }

    write::enqueue(
        conn,
        &OutboxRow {
            op: write::OP_FLAGS.to_string(),
            payload: serde_json::to_string(&FlagsPayload {
                ids: ids.to_vec(),
                patch: patch.clone(),
            })
            .map_err(|e| e.to_string())?,
            thread_key: thread_key.map(str::to_string),
            created_at: now,
            ..OutboxRow::default()
        },
    )
    .map(|_| ())
}

pub fn queue_labels(
    conn: &rusqlite::Connection,
    ids: &[String],
    add: &[String],
    remove: &[String],
    thread_key: Option<&str>,
) -> Result<(), String> {
    if ids.is_empty() || (add.is_empty() && remove.is_empty()) {
        return Ok(());
    }
    let now = write::now_ms();
    let mut sorted = ids.to_vec();
    sorted.sort();

    if let Some(row) = write::coalescable(conn, write::OP_LABELS, now)? {
        if let Ok(held) = serde_json::from_str::<LabelsPayload>(&row.payload) {
            let mut theirs = held.ids.clone();
            theirs.sort();
            if theirs == sorted {
                let mut folded = LabelsPayload {
                    ids: held.ids,
                    add: held.add,
                    remove: held.remove,
                };
                for label in add {
                    folded.remove.retain(|held| held != label);
                    if !folded.add.contains(label) {
                        folded.add.push(label.clone());
                    }
                }
                for label in remove {
                    folded.add.retain(|held| held != label);
                    if !folded.remove.contains(label) {
                        folded.remove.push(label.clone());
                    }
                }
                return write::enqueue(
                    conn,
                    &OutboxRow {
                        payload: serde_json::to_string(&folded).map_err(|e| e.to_string())?,
                        ..row
                    },
                )
                .map(|_| ());
            }
        }
    }

    write::enqueue(
        conn,
        &OutboxRow {
            op: write::OP_LABELS.to_string(),
            payload: serde_json::to_string(&LabelsPayload {
                ids: ids.to_vec(),
                add: add.to_vec(),
                remove: remove.to_vec(),
            })
            .map_err(|e| e.to_string())?,
            thread_key: thread_key.map(str::to_string),
            created_at: now,
            ..OutboxRow::default()
        },
    )
    .map(|_| ())
}

fn next_ready(conn: &rusqlite::Connection, now: i64) -> Result<Option<OutboxRow>, String> {
    Ok(write::outbox_rows(conn, 50)?
        .into_iter()
        .find(|row| row.hold_until <= now))
}

/// How long to wait after a failure. Truncated exponential, and a rate limit is believed rather
/// than guessed at, because the provider knows and we do not.
fn backoff_ms(attempts: i64, error: &ProviderError) -> i64 {
    match error {
        ProviderError::RateLimited { retry_after_ms } => *retry_after_ms as i64,
        _ => MAX_BACKOFF_MS.min(1_000 << attempts.clamp(0, 6)),
    }
}

async fn push<S: Store>(
    store: &S,
    remote: &dyn Remote,
    row: &OutboxRow,
) -> Result<(), ProviderError> {
    match row.op.as_str() {
        write::OP_FLAGS => {
            let payload: FlagsPayload = serde_json::from_str(&row.payload)
                .map_err(|e| ProviderError::Other(e.to_string()))?;
            remote.set_flags(&payload.ids, &payload.patch).await
        }
        write::OP_LABELS => {
            let payload: LabelsPayload = serde_json::from_str(&row.payload)
                .map_err(|e| ProviderError::Other(e.to_string()))?;
            remote
                .set_labels(&payload.ids, &payload.add, &payload.remove)
                .await
        }
        write::OP_SEND => crate::send::push(store, remote, row).await,
        other => Err(ProviderError::Other(format!("no such outbox operation: {other}"))),
    }
}

/// Whether a failure is about the connection or the account rather than about this row. The row
/// is kept, and the rows behind it wait, because they would fail the same way.
fn transient(error: &ProviderError) -> bool {
    !matches!(error, ProviderError::NotFound | ProviderError::Other(_))
}

fn flag_verbs(patch: &FlagPatch) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (value, on, off) in [
        (patch.trashed, "Trashing", "Restoring from the trash"),
        (patch.spam, "Marking as spam", "Marking as not spam"),
        (patch.archived, "Archiving", "Moving to the Inbox"),
        (patch.starred, "Starring", "Unstarring"),
        (patch.seen, "Marking read", "Marking unread"),
    ] {
        match value {
            Some(true) => out.push(on),
            Some(false) => out.push(off),
            None => {}
        }
    }
    if out.is_empty() {
        out.push("Changing");
    }
    out
}

/// The change a row carries, as a person would name it: "Archiving 3 messages", "Starring and
/// marking read 1 message".
fn describe(row: &OutboxRow) -> String {
    let (verbs, count) = match row.op.as_str() {
        write::OP_FLAGS => match serde_json::from_str::<FlagsPayload>(&row.payload) {
            Ok(payload) => (flag_verbs(&payload.patch), payload.ids.len()),
            Err(_) => (vec!["Changing"], 0),
        },
        write::OP_LABELS => match serde_json::from_str::<LabelsPayload>(&row.payload) {
            Ok(payload) => (vec!["Changing labels on"], payload.ids.len()),
            Err(_) => (vec!["Changing labels on"], 0),
        },
        _ => (vec!["Changing"], 0),
    };
    let mut words: Vec<String> = verbs
        .iter()
        .enumerate()
        .map(|(n, verb)| {
            if n == 0 {
                verb.to_string()
            } else {
                verb.to_lowercase()
            }
        })
        .collect();
    let last = words.pop().unwrap_or_default();
    let doing = if words.is_empty() {
        last
    } else {
        format!("{} and {last}", words.join(", "))
    };
    let what = match count {
        0 => "some messages".to_string(),
        1 => "1 message".to_string(),
        n => format!("{n} messages"),
    };
    format!("{doing} {what}")
}

/// Drains what it can inside the budget, and says what it gave up on.
///
/// A row the provider could not be reached for, or refused for a reason that is about the account,
/// is deferred with the reason on it and the drain stops: the next row is the same account and
/// would fail the same way. A row the provider refused for a reason that is about the row is
/// nobody else's problem and does not hold the queue: it is deferred once in case the refusal was
/// a fluke, and dropped on the next refusal with its reason recorded for the engine to say once. A
/// message the provider no longer has needs no change at all, so that row is simply done.
///
/// A send is none of this. Its bookkeeping is `crate::send`'s and every failure defers it there.
pub async fn drain_reporting<S: Store>(
    store: &S,
    remote: &dyn Remote,
    deadline_ms: i64,
) -> Drained {
    let mut drained = Drained::default();
    loop {
        if write::now_ms() >= deadline_ms {
            break;
        }
        let row = match store.with(|conn| next_ready(conn, write::now_ms())) {
            Ok(row) => row,
            Err(e) => {
                drained.outcome.error = Some(ProviderError::Other(e));
                break;
            }
        };
        let Some(row) = row else { break };

        let error = match push(store, remote, &row).await {
            Ok(()) => {
                if let Err(e) = store.with(|conn| write::dequeue(conn, &row.id)) {
                    drained.outcome.error = Some(ProviderError::Other(e));
                    break;
                }
                drained.outcome.changed = true;
                continue;
            }
            Err(error) => error,
        };

        // A send another drain already holds is not a failure and leaves no mark on the row: the
        // drain holding it is the one that will say what happened to it.
        if crate::send::taken(&error) {
            break;
        }
        let until = write::now_ms() + backoff_ms(row.attempts, &error);
        let reason = error.to_string();
        if row.op == write::OP_SEND {
            // A send counted this attempt when it leased the row, so deferring it must not count
            // a second one.
            let _ = store.with(|conn| crate::send::deferred(conn, &row.id, until, &reason));
            drained.outcome.error = Some(error);
            break;
        }
        if error == ProviderError::NotFound {
            let _ = store.with(|conn| write::dequeue(conn, &row.id));
            drained.outcome.changed = true;
            continue;
        }
        if transient(&error) {
            let _ = store.with(|conn| write::defer(conn, &row.id, until, &reason));
            drained.outcome.error = Some(error);
            break;
        }
        if row.attempts + 1 < MAX_WRITE_ATTEMPTS {
            let _ = store.with(|conn| write::defer(conn, &row.id, until, &reason));
            continue;
        }
        let _ = store.with(|conn| write::dequeue(conn, &row.id));
        drained.outcome.changed = true;
        drained
            .dropped
            .push(format!("{} was refused and dropped: {reason}", describe(&row)));
    }
    drained
}

/// The drain without its report, for the flush at quit, where nobody is left to tell.
pub async fn drain<S: Store>(store: &S, remote: &dyn Remote, deadline_ms: i64) -> Outcome {
    drain_reporting(store, remote, deadline_ms).await.outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags_row(patch: FlagPatch, ids: &[&str]) -> OutboxRow {
        OutboxRow {
            op: write::OP_FLAGS.to_string(),
            payload: serde_json::to_string(&FlagsPayload {
                ids: ids.iter().map(|id| id.to_string()).collect(),
                patch,
            })
            .expect("json"),
            ..OutboxRow::default()
        }
    }

    #[test]
    fn a_row_is_described_by_what_it_does_and_to_how_many() {
        assert_eq!(
            describe(&flags_row(
                FlagPatch {
                    archived: Some(true),
                    ..FlagPatch::default()
                },
                &["a", "b", "c"]
            )),
            "Archiving 3 messages"
        );
        assert_eq!(
            describe(&flags_row(
                FlagPatch {
                    starred: Some(true),
                    seen: Some(true),
                    ..FlagPatch::default()
                },
                &["a"]
            )),
            "Starring and marking read 1 message"
        );
        assert_eq!(
            describe(&flags_row(
                FlagPatch {
                    trashed: Some(false),
                    archived: Some(false),
                    seen: Some(false),
                    ..FlagPatch::default()
                },
                &["a", "b"]
            )),
            "Restoring from the trash, moving to the inbox and marking unread 2 messages"
        );
        let labels = OutboxRow {
            op: write::OP_LABELS.to_string(),
            payload: serde_json::to_string(&LabelsPayload {
                ids: vec!["a".into()],
                add: vec!["L1".into()],
                remove: vec![],
            })
            .expect("json"),
            ..OutboxRow::default()
        };
        assert_eq!(describe(&labels), "Changing labels on 1 message");
    }

    #[test]
    fn only_the_rows_own_refusals_are_permanent() {
        assert!(transient(&ProviderError::Network("tunnel".into())));
        assert!(transient(&ProviderError::RateLimited { retry_after_ms: 1 }));
        assert!(transient(&ProviderError::Auth("revoked".into())));
        assert!(transient(&ProviderError::Scope("gmail.modify".into())));
        assert!(!transient(&ProviderError::NotFound));
        assert!(!transient(&ProviderError::Other("Gmail label change failed (400): no".into())));
    }
}
