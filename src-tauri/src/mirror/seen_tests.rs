// Opened means seen, and what that does and does not touch.
//
// Over a pair of in-memory databases with a thread written the way hydration writes one, so the
// thread-level rule under test is the real `refresh_thread` and the groups are the real
// `threads_list`, not rows inserted by hand to match.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Place, ThreadPage, ThreadQuery};
use crate::mirror::{read, write};
use crate::provider::RawHeaders;
use crate::sync::outbox::FlagsPayload;

use super::seen_on_open;

const THREAD: &str = "seen-test-t1";
const KEY: &str = "seen-test-m1@example.com";
const FIRST: &str = "seen-test-m1";
const SECOND: &str = "seen-test-m2";

fn headers(id: &str, message_id: &str, reply_to: Option<&str>, date_ms: i64, labels: &[&str]) -> RawHeaders {
    let mut headers = vec![
        ("Message-ID".to_string(), format!("<{message_id}>")),
        ("From".to_string(), "Ana <ana@example.com>".to_string()),
        ("To".to_string(), "PJ <pj@example.com>".to_string()),
        ("Subject".to_string(), "The lease".to_string()),
    ];
    if let Some(parent) = reply_to {
        headers.push(("In-Reply-To".to_string(), format!("<{parent}>")));
        headers.push(("References".to_string(), format!("<{parent}>")));
    }
    RawHeaders {
        id: id.to_string(),
        thread_id: THREAD.to_string(),
        label_ids: labels.iter().map(|l| l.to_string()).collect(),
        internal_date_ms: date_ms,
        size: 1_000,
        snippet: String::new(),
        headers,
    }
}

/// A two message thread from Ana, routed to the Inbox, with the labels each message arrived with.
fn mailbox(first: &[&str], second: &[&str]) -> Connection {
    let conn = db::memory().expect("a pair of in-memory databases");
    crate::state::write::set_rule(&conn, "ana@example.com", false, Destination::Inbox, None)
        .expect("rule");
    write::upsert_message(&conn, &headers(FIRST, KEY, None, 1_000, first), false).expect("m1");
    write::upsert_message(
        &conn,
        &headers(SECOND, "seen-test-m2@example.com", Some(KEY), 2_000, second),
        false,
    )
    .expect("m2");
    write::refresh_thread(&conn, THREAD).expect("thread");
    conn
}

fn message(conn: &Connection, id: &str) -> (bool, Vec<String>) {
    conn.query_row(
        "SELECT seen, labels FROM messages WHERE id = ?1",
        [id],
        |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, String>(1)?)),
    )
    .map(|(seen, labels)| (seen, serde_json::from_str(&labels).unwrap_or_default()))
    .expect("message")
}

fn thread_unseen(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT unseen FROM threads WHERE provider_thread_id = ?1",
        [THREAD],
        |row| row.get::<_, i64>(0),
    )
    .expect("thread")
        != 0
}

fn inbox(conn: &Connection) -> ThreadPage {
    read::threads_list(
        conn,
        "acct",
        "hue-1",
        &ThreadQuery {
            account_id: None,
            place: Place::Inbox,
            label_id: None,
            query: None,
            limit: 50,
            cursor: None,
        },
        write::now_ms(),
    )
    .expect("the Inbox")
}

fn queued_flags(conn: &Connection) -> Vec<FlagsPayload> {
    write::outbox_rows(conn, 50)
        .expect("outbox")
        .into_iter()
        .filter(|row| row.op == write::OP_FLAGS)
        .map(|row| serde_json::from_str(&row.payload).expect("a flags payload"))
        .collect()
}

/// Whether the mirror's undo stack holds anything about this file's messages. By id rather than by
/// depth, because the stack is process wide and the rest of the suite pushes to it in parallel.
fn undo_mentions_these(conn: &Connection) -> bool {
    let _ = conn;
    let stack = super::undo_stack().lock().expect("undo stack");
    stack.iter().any(|entry| {
        entry
            .prior
            .iter()
            .any(|(_, prior)| prior.iter().any(|flags| flags.id.starts_with("seen-test-")))
    })
}

#[test]
fn opening_marks_every_unseen_message_seen_and_moves_the_thread_out_of_new_for_you() {
    let conn = mailbox(&["INBOX", "UNREAD"], &["INBOX", "UNREAD"]);
    assert!(thread_unseen(&conn));
    assert_eq!(inbox(&conn).threads[0].group, "new");
    assert_eq!(read::inbox_unseen(&conn).expect("badge"), 1);

    assert!(seen_on_open(&conn, KEY).expect("open"));

    for id in [FIRST, SECOND] {
        let (seen, labels) = message(&conn, id);
        assert!(seen, "{id} is seen");
        assert!(!labels.iter().any(|l| l == write::LABEL_UNREAD), "{id} lost UNREAD");
    }
    assert!(!thread_unseen(&conn), "the thread is seen once every message is");
    assert_eq!(inbox(&conn).threads[0].group, "seen");
    assert_eq!(read::inbox_unseen(&conn).expect("badge"), 0, "the badge counts one fewer");

    // One row to the provider, covering both messages, saying seen and nothing else.
    let queued = queued_flags(&conn);
    assert_eq!(queued.len(), 1);
    let mut ids = queued[0].ids.clone();
    ids.sort();
    assert_eq!(ids, vec![FIRST.to_string(), SECOND.to_string()]);
    assert_eq!(queued[0].patch.seen, Some(true));
    assert_eq!(
        (queued[0].patch.starred, queued[0].patch.archived, queued[0].patch.trashed, queued[0].patch.spam),
        (None, None, None, None)
    );
}

#[test]
fn only_the_unseen_messages_are_written_and_queued() {
    let conn = mailbox(&["INBOX"], &["INBOX", "UNREAD"]);
    let before = message(&conn, FIRST);

    assert!(seen_on_open(&conn, KEY).expect("open"));

    assert_eq!(message(&conn, FIRST), before, "a message already seen is not rewritten");
    assert!(message(&conn, SECOND).0);
    let queued = queued_flags(&conn);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].ids, vec![SECOND.to_string()]);
}

#[test]
fn a_thread_already_seen_writes_nothing_queues_nothing_and_says_so() {
    let conn = mailbox(&["INBOX"], &["INBOX"]);
    assert!(!thread_unseen(&conn));

    assert!(!seen_on_open(&conn, KEY).expect("open"));

    assert!(queued_flags(&conn).is_empty());
    assert_eq!(write::pending_writes(&conn).expect("outbox"), 0);
}

#[test]
fn opening_twice_is_one_row_to_the_provider() {
    let conn = mailbox(&["INBOX", "UNREAD"], &["INBOX", "UNREAD"]);
    assert!(seen_on_open(&conn, KEY).expect("first open"));
    assert!(!seen_on_open(&conn, KEY).expect("second open"));
    assert_eq!(queued_flags(&conn).len(), 1);
}

/// `z` after opening a thread has to take back the archive before it, not the reading of this
/// one, so an open never reaches the undo stack.
#[test]
fn opening_is_not_something_z_takes_back() {
    let conn = mailbox(&["INBOX", "UNREAD"], &["INBOX", "UNREAD"]);
    assert!(seen_on_open(&conn, KEY).expect("open"));
    assert!(!undo_mentions_these(&conn));
}

/// Ignore is about New for you and notifications, not about read state: the thread was never in
/// New for you, and opening it still marks it seen the way any other is.
#[test]
fn an_ignored_thread_is_marked_seen_on_open_like_any_other() {
    let conn = mailbox(&["INBOX", "UNREAD"], &["INBOX", "UNREAD"]);
    crate::state::write::set_thread_flags(&conn, KEY, Some(true), None).expect("ignore");
    assert_eq!(inbox(&conn).threads[0].group, "seen", "ignored is never new");
    assert_eq!(read::inbox_unseen(&conn).expect("badge"), 0);

    assert!(seen_on_open(&conn, KEY).expect("open"));

    assert!(!thread_unseen(&conn));
    assert_eq!(queued_flags(&conn).len(), 1);
}

/// The reverse race. A sync pass that fetched its metadata before the open landed still carries
/// `UNREAD`, and used to put the dot back until the outbox row reached the server and the next
/// pass took it off again. A change this device has queued is the truth about those labels until
/// the server has heard it; once it has, the server's word lands like any other.
#[test]
fn what_the_provider_says_lands_under_what_is_still_queued() {
    let conn = mailbox(&["INBOX"], &["INBOX", "UNREAD"]);
    assert!(seen_on_open(&conn, KEY).expect("open"));

    let stale = vec!["INBOX".to_string(), "UNREAD".to_string()];
    write::set_labels(&conn, SECOND, &stale).expect("a stale word from the provider");
    write::refresh_thread(&conn, THREAD).expect("thread");
    let (seen, labels) = message(&conn, SECOND);
    assert!(seen, "the open wins while its row is still queued");
    assert_eq!(labels, vec!["INBOX".to_string()]);
    assert!(!thread_unseen(&conn));

    // Drained: the row is gone, so a later UNREAD from the provider is somebody else marking it
    // unread on another device, and it lands.
    for row in write::outbox_rows(&conn, 50).expect("outbox") {
        write::dequeue(&conn, &row.id).expect("dequeue");
    }
    write::set_labels(&conn, SECOND, &stale).expect("the provider's word");
    write::refresh_thread(&conn, THREAD).expect("thread");
    assert!(!message(&conn, SECOND).0);
    assert!(thread_unseen(&conn));
}

/// A reversal is the one write that must overrule what is queued, which is why `undo_apply` lands
/// through `apply_labels` rather than `set_labels`.
#[test]
fn a_reversal_overrules_what_is_queued() {
    let conn = mailbox(&["INBOX"], &["INBOX", "UNREAD"]);
    assert!(seen_on_open(&conn, KEY).expect("open"));

    write::apply_labels(&conn, &[SECOND.to_string()], &["UNREAD".to_string()], &[]).expect("undo");
    let (seen, labels) = message(&conn, SECOND);
    assert!(!seen);
    assert!(labels.iter().any(|l| l == write::LABEL_UNREAD));
}
