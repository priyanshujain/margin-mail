// Moving one thread and moving a sender, read back through the list pages they change.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Pile, Place, ThreadQuery};
use crate::mirror;
use crate::state;

const NOW: i64 = 2_000_000_000_000;

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
    key
}

fn keys_in(conn: &Connection, place: Place) -> Vec<String> {
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
        .into_iter()
        .map(|thread| thread.key)
        .collect()
}

#[test]
fn one_thread_moves_and_the_senders_other_threads_stay() {
    let conn = open();
    state::write::set_rule(&conn, "fund@example.org", false, Destination::Inbox, None).unwrap();
    let moved = arrive(&conn, "fund@example.org", 100);
    let stays = arrive(&conn, "fund@example.org", 200);

    super::move_threads(&conn, std::slice::from_ref(&moved), Destination::PaperTrail).unwrap();

    assert_eq!(keys_in(&conn, Place::PaperTrail), vec![moved]);
    assert_eq!(keys_in(&conn, Place::Inbox), vec![stays]);
}

#[test]
fn a_moved_thread_leaves_the_screener_and_its_sender_still_waits() {
    let conn = open();
    let moved = arrive(&conn, "new@example.org", 100);
    let waiting = arrive(&conn, "new@example.org", 200);

    super::move_threads(&conn, std::slice::from_ref(&moved), Destination::Feed).unwrap();

    assert_eq!(keys_in(&conn, Place::Feed), vec![moved]);
    assert_eq!(keys_in(&conn, Place::Screener), vec![waiting]);
}

#[test]
fn everything_from_a_sender_takes_back_the_threads_moved_by_hand() {
    let conn = open();
    state::write::set_rule(&conn, "fund@example.org", false, Destination::Inbox, None).unwrap();
    let first = arrive(&conn, "fund@example.org", 100);
    let second = arrive(&conn, "fund@example.org", 200);
    super::move_threads(&conn, std::slice::from_ref(&first), Destination::Feed).unwrap();

    let (_, senders) =
        super::move_senders(&conn, "acct", std::slice::from_ref(&second), Destination::PaperTrail)
            .unwrap();

    assert_eq!(senders, vec!["fund@example.org".to_string()]);
    assert_eq!(keys_in(&conn, Place::PaperTrail), vec![second, first]);
    assert!(keys_in(&conn, Place::Feed).is_empty());
}

#[test]
fn undoing_a_sender_move_puts_back_the_rule_and_the_threads_moved_by_hand() {
    let conn = open();
    state::write::set_rule(&conn, "fund@example.org", false, Destination::Inbox, None).unwrap();
    let first = arrive(&conn, "fund@example.org", 100);
    let second = arrive(&conn, "fund@example.org", 200);
    super::move_threads(&conn, std::slice::from_ref(&first), Destination::Feed).unwrap();

    let (before, _) =
        super::move_senders(&conn, "acct", std::slice::from_ref(&second), Destination::PaperTrail)
            .unwrap();
    super::restore(&conn, &before).unwrap();

    assert_eq!(keys_in(&conn, Place::Feed), vec![first]);
    assert_eq!(keys_in(&conn, Place::Inbox), vec![second]);
}

#[test]
fn undoing_a_sender_move_leaves_a_domain_rule_in_charge_again() {
    let conn = open();
    state::write::set_rule(&conn, "example.org", true, Destination::Feed, None).unwrap();
    let key = arrive(&conn, "fund@example.org", 100);

    let (before, _) =
        super::move_senders(&conn, "acct", std::slice::from_ref(&key), Destination::PaperTrail)
            .unwrap();
    assert_eq!(keys_in(&conn, Place::PaperTrail), vec![key.clone()]);

    super::restore(&conn, &before).unwrap();
    assert_eq!(keys_in(&conn, Place::Feed), vec![key]);
}

#[test]
fn a_moved_thread_comes_off_its_pile_and_undo_puts_it_back() {
    let conn = open();
    state::write::set_rule(&conn, "fund@example.org", false, Destination::Inbox, None).unwrap();
    let key = arrive(&conn, "fund@example.org", 100);
    state::write::set_pile(&conn, &key, Pile::SetAside).unwrap();

    let before =
        super::move_threads(&conn, std::slice::from_ref(&key), Destination::PaperTrail).unwrap();
    assert_eq!(keys_in(&conn, Place::PaperTrail), vec![key.clone()]);
    assert!(keys_in(&conn, Place::SetAside).is_empty());

    super::restore(&conn, &before).unwrap();
    assert_eq!(keys_in(&conn, Place::SetAside), vec![key]);
    assert!(keys_in(&conn, Place::PaperTrail).is_empty());
}

#[test]
fn a_thread_place_survives_a_replay_of_the_journal() {
    let conn = open();
    let key = arrive(&conn, "fund@example.org", 100);
    super::move_threads(&conn, std::slice::from_ref(&key), Destination::PaperTrail).unwrap();

    state::journal::replay(&conn).unwrap();

    assert_eq!(
        state::read::thread_place(&conn, &key).unwrap(),
        Some(Destination::PaperTrail)
    );
}
