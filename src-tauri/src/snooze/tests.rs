// Snooze, over a pair of in-memory databases and a fixed now.
//
// Nothing here runs a clock. The return times are computed from a moment the test names and read
// back exactly, and the evaluation is handed the moment it is supposed to be evaluating at, which
// is the only way "returns what is due and nothing else" is a testable sentence.

use chrono::NaiveDate;
use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Place, SnoozeKind, SnoozeTimes, ThreadQuery, ThreadSummary};
use crate::mirror;
use crate::settings;
use crate::state;

const NOW: i64 = 2_000_000_000_000;
const DAY: i64 = 86_400_000;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

fn arrive(conn: &Connection, address: &str, at: i64) -> String {
    let key = format!("<{address}-{at}@example>");
    let tid = format!("t-{address}-{at}");
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox)
         VALUES (?1, ?2, ?3, 1, 1, 'Hello', 'snippet', 'Someone', ?4, 1)",
        rusqlite::params![tid, key, at, address],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, labels)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'Hello', 'snippet', 1, '[\"INBOX\"]')",
        rusqlite::params![format!("m-{tid}"), tid, key, at, address],
    )
    .expect("a message");
    state::write::set_rule(conn, address, false, Destination::Inbox, None).expect("a rule");
    key
}

/// Another message in the thread. `mine` is what the account sent, which is not a reply to itself.
fn append(conn: &Connection, key: &str, at: i64, mine: bool) {
    let tid: String = conn
        .query_row(
            "SELECT provider_thread_id FROM threads WHERE thread_key = ?1",
            [key],
            |row| row.get(0),
        )
        .expect("the thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, sent, labels)
         VALUES (?1, ?2, ?3, ?1, ?4, ?5, 'Hello', 'snippet', 1, ?6, '[\"INBOX\"]')",
        rusqlite::params![
            format!("m-{tid}-{at}"),
            tid,
            key,
            at,
            if mine { "you@example.com" } else { "them@example.org" },
            mine as i64,
        ],
    )
    .expect("another message");
    conn.execute(
        "UPDATE threads SET latest_ms = ?2, message_count = message_count + 1, unseen = 1
          WHERE thread_key = ?1",
        rusqlite::params![key, at],
    )
    .expect("the thread moved on");
}

fn page(conn: &Connection, place: Place) -> Vec<ThreadSummary> {
    let query = ThreadQuery {
        account_id: None,
        place,
        label_id: None,
        query: None,
        limit: 50,
        cursor: None,
    };
    mirror::read::threads_list(conn, "acct", "hue-1", &query, NOW)
        .expect("a page")
        .threads
}

fn keys_in(conn: &Connection, place: Place) -> Vec<String> {
    page(conn, place).into_iter().map(|t| t.key).collect()
}

// ---------------------------------------------------------------------------------------------
// The picker
// ---------------------------------------------------------------------------------------------

fn at_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|day| day.and_hms_opt(hour, minute, 0))
        .expect("a moment")
        .and_utc()
        .timestamp_millis()
}

#[test]
fn each_kind_computes_its_own_return_time_from_a_fixed_now() {
    // A Wednesday, mid morning.
    let now = at_utc(2024, 1, 3, 10, 0);
    let times = settings::defaults().snooze_times;

    assert_eq!(
        super::return_at(SnoozeKind::LaterToday, now, &times, 0),
        at_utc(2024, 1, 3, 13, 0)
    );
    assert_eq!(
        super::return_at(SnoozeKind::Tomorrow, now, &times, 0),
        at_utc(2024, 1, 4, 8, 0)
    );
    assert_eq!(
        super::return_at(SnoozeKind::Weekend, now, &times, 0),
        at_utc(2024, 1, 6, 9, 0),
        "this weekend is the coming Saturday"
    );
    assert_eq!(
        super::return_at(SnoozeKind::NextWeek, now, &times, 0),
        at_utc(2024, 1, 8, 8, 0),
        "next week is the coming Monday"
    );
    // Both of these name their own moment on the picker, and the picker opens on tomorrow.
    assert_eq!(
        super::return_at(SnoozeKind::Date, now, &times, 0),
        at_utc(2024, 1, 4, 8, 0)
    );
    assert_eq!(
        super::return_at(SnoozeKind::IfNoReply, now, &times, 0),
        at_utc(2024, 1, 4, 8, 0)
    );
}

#[test]
fn a_time_of_day_is_a_local_one() {
    let now = at_utc(2024, 1, 3, 10, 0);
    let times = SnoozeTimes {
        later_today_hours: 3,
        tomorrow_at: 8 * 60,
        weekend_at: 9 * 60,
        next_week_at: 8 * 60,
    };
    // Five and a half hours east: eight in the morning there is half past two here.
    assert_eq!(
        super::return_at(SnoozeKind::Tomorrow, now, &times, 330),
        at_utc(2024, 1, 4, 2, 30)
    );
}

#[test]
fn a_weekend_asked_for_after_saturday_morning_means_the_next_one() {
    let times = settings::defaults().snooze_times;
    // Saturday afternoon, past the hour it would have come back at.
    let now = at_utc(2024, 1, 6, 15, 0);
    assert_eq!(
        super::return_at(SnoozeKind::Weekend, now, &times, 0),
        at_utc(2024, 1, 13, 9, 0)
    );
}

// ---------------------------------------------------------------------------------------------
// The evaluation
// ---------------------------------------------------------------------------------------------

#[test]
fn an_evaluation_returns_what_is_due_and_nothing_else() {
    let conn = open();
    let due = arrive(&conn, "maya@example.org", 100);
    let later = arrive(&conn, "ana@example.org", 200);
    super::set(&conn, std::slice::from_ref(&due), SnoozeKind::Tomorrow, NOW - 1_000).expect("due");
    super::set(&conn, std::slice::from_ref(&later), SnoozeKind::Tomorrow, NOW + DAY)
        .expect("later");

    assert!(keys_in(&conn, Place::Inbox).is_empty(), "a snoozed thread is away");
    assert_eq!(keys_in(&conn, Place::Snoozed).len(), 2);

    let back = super::evaluate(&conn, NOW).expect("an evaluation");
    assert_eq!(back, vec![due.clone()]);

    let inbox = page(&conn, Place::Inbox);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].key, due);
    assert_eq!(inbox[0].group, "back", "a thread that came back waits in Back");
    assert_eq!(keys_in(&conn, Place::Snoozed), vec![later]);

    assert!(
        super::evaluate(&conn, NOW).expect("a second pass").is_empty(),
        "a thread that came back must not come back twice"
    );

    super::opened(&conn, &due).expect("opened");
    let inbox = page(&conn, Place::Inbox);
    assert_eq!(inbox[0].group, "new", "opening it takes it out of Back");
}

#[test]
fn if_no_reply_returns_a_thread_nobody_answered() {
    let conn = open();
    let key = arrive(&conn, "client@example.org", 100);
    super::set(&conn, std::slice::from_ref(&key), SnoozeKind::IfNoReply, NOW - 1_000)
        .expect("a reminder");
    // A follow up of your own is not somebody answering you.
    append(&conn, &key, 150, true);

    assert_eq!(super::evaluate(&conn, NOW).expect("an evaluation"), vec![key]);
}

#[test]
fn a_reply_cancels_the_reminder_instead_of_returning_the_thread() {
    let conn = open();
    let key = arrive(&conn, "client@example.org", 100);
    super::set(&conn, std::slice::from_ref(&key), SnoozeKind::IfNoReply, NOW - 1_000)
        .expect("a reminder");
    append(&conn, &key, 150, false);

    assert!(
        super::evaluate(&conn, NOW).expect("an evaluation").is_empty(),
        "somebody wrote, so there is nothing to remind about"
    );
    assert!(
        state::read::snooze_of(&conn, &key).expect("the row").is_none(),
        "the reminder cancels itself"
    );
    assert_eq!(
        keys_in(&conn, Place::Inbox),
        vec![key],
        "and the reply lands as normal"
    );
}

#[test]
fn a_thread_that_comes_back_late_still_knows_when_it_was_due() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", 100);
    let due_at = NOW - 2 * DAY;
    super::set(&conn, std::slice::from_ref(&key), SnoozeKind::Tomorrow, due_at).expect("a snooze");

    super::evaluate(&conn, NOW).expect("an evaluation");

    assert_eq!(
        super::returned_due(&conn, &key).expect("the returned row"),
        Some(due_at),
        "a thread due yesterday says so rather than pretending it returned on time"
    );
}

#[test]
fn clearing_a_snooze_by_hand_puts_the_thread_back_in_its_list() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", 100);
    super::set(&conn, std::slice::from_ref(&key), SnoozeKind::Tomorrow, NOW + DAY).expect("snooze");
    assert!(keys_in(&conn, Place::Inbox).is_empty());

    let before = super::clear(&conn, std::slice::from_ref(&key)).expect("cleared");
    assert_eq!(keys_in(&conn, Place::Inbox), vec![key.clone()]);
    assert!(keys_in(&conn, Place::Snoozed).is_empty());

    for held in &before {
        super::restore(&conn, held).expect("put back");
    }
    assert_eq!(keys_in(&conn, Place::Snoozed), vec![key]);
}
