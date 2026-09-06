// The two piles, over a pair of in-memory databases.
//
// Threads are inserted with SQL and read back through a list page, so what is under test is the
// place clause and the stack rather than the sync engine that would otherwise have built them.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Pile, Place, ThreadQuery};
use crate::mirror;
use crate::state;

const NOW: i64 = 2_000_000_000_000;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

fn arrive(conn: &Connection, address: &str, subject: &str, at: i64) -> String {
    let key = format!("<{address}-{at}@example>");
    let tid = format!("t-{address}-{at}");
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 'snippet', 'Someone', ?5, 1)",
        rusqlite::params![tid, key, at, subject, address],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, labels)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 'snippet', 1, '[\"INBOX\"]')",
        rusqlite::params![format!("m-{tid}"), tid, key, at, address, subject],
    )
    .expect("a message");
    state::write::set_rule(conn, address, false, Destination::Inbox, None).expect("a rule");
    key
}

/// Three threads from three senders, oldest first, which is the order they go on a pile in.
fn three(conn: &Connection) -> Vec<String> {
    [
        ("one@example.org", 100),
        ("two@example.org", 200),
        ("three@example.org", 300),
    ]
    .into_iter()
    .map(|(address, at)| arrive(conn, address, "Hello", at))
    .collect()
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
fn a_pile_takes_a_thread_out_of_its_list_and_the_place_shows_it() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "Hello", 100);
    assert_eq!(keys_in(&conn, Place::Inbox), vec![key.clone()]);

    super::toggle(&conn, std::slice::from_ref(&key), Pile::ReplyLater).expect("on");
    assert_eq!(keys_in(&conn, Place::ReplyLater), vec![key.clone()]);
    assert!(
        keys_in(&conn, Place::Inbox).is_empty(),
        "a piled thread does not render in the list it came from"
    );
    assert!(keys_in(&conn, Place::SetAside).is_empty());

    super::toggle(&conn, std::slice::from_ref(&key), Pile::ReplyLater).expect("off");
    assert!(keys_in(&conn, Place::ReplyLater).is_empty());
    assert_eq!(
        keys_in(&conn, Place::Inbox),
        vec![key],
        "coming off a pile puts a thread back where it came from, which needs no record"
    );
}

#[test]
fn the_other_key_moves_a_thread_across_rather_than_onto_both() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "Hello", 100);

    super::toggle(&conn, std::slice::from_ref(&key), Pile::ReplyLater).expect("reply later");
    super::toggle(&conn, std::slice::from_ref(&key), Pile::SetAside).expect("set aside");

    assert_eq!(keys_in(&conn, Place::SetAside), vec![key]);
    assert!(keys_in(&conn, Place::ReplyLater).is_empty());
}

#[test]
fn a_pile_is_a_stack_and_its_order_is_the_order_things_went_on_it() {
    let conn = open();
    let keys = three(&conn);

    for key in &keys {
        super::toggle(&conn, std::slice::from_ref(key), Pile::ReplyLater).expect("on");
    }

    let mut expected = keys.clone();
    expected.reverse();
    assert_eq!(
        state::read::pile_keys(&conn, Pile::ReplyLater).expect("the stack"),
        expected,
        "the last card put on the pile is the one on top"
    );
}

#[test]
fn a_toggle_over_a_mixed_selection_remembers_each_card_where_it_was() {
    let conn = open();
    let keys = three(&conn);
    for key in &keys {
        super::toggle(&conn, std::slice::from_ref(key), Pile::ReplyLater).expect("on");
    }
    let stack = state::read::pile_keys(&conn, Pile::ReplyLater).expect("the stack");

    let middle = keys[1].clone();
    let toggled =
        super::toggle(&conn, std::slice::from_ref(&middle), Pile::ReplyLater).expect("off");
    assert_eq!(toggled.removed, 1);

    for (key, before) in &toggled.before {
        match before {
            Some((pile, position)) => {
                state::write::set_pile_at(&conn, key, *pile, *position).expect("back")
            }
            None => state::write::clear_pile(&conn, key).expect("off"),
        }
    }
    assert_eq!(
        state::read::pile_keys(&conn, Pile::ReplyLater).expect("the stack"),
        stack,
        "a card put back goes where it was rather than on top"
    );
}
