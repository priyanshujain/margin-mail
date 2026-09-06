// Notes, names, merges and the two switches, over a pair of in-memory databases.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Place, ThreadQuery, ThreadSummary};
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

/// A new message in an existing thread, appended the way a sync would append it.
fn append(conn: &Connection, key: &str, at: i64) {
    let tid: String = conn
        .query_row(
            "SELECT provider_thread_id FROM threads WHERE thread_key = ?1",
            [key],
            |row| row.get(0),
        )
        .expect("the thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, labels)
         VALUES (?1, ?2, ?3, ?1, ?4, 'them@example.org', 'Hello', 'snippet', 1, '[\"INBOX\"]')",
        rusqlite::params![format!("m-{tid}-{at}"), tid, key, at],
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
// Notes
// ---------------------------------------------------------------------------------------------

#[test]
fn a_note_round_trips_and_a_second_one_is_a_second_note() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "The kitchen", 100);

    let first = super::add_note(&conn, &key, "Chase this on Friday").expect("a note");
    assert_eq!(first.body, "Chase this on Friday");
    assert_eq!(first.thread_key, key);
    assert_eq!(
        first.after_message_id.as_deref(),
        Some("m-t-maya@example.org-100"),
        "a note sits after the message that was latest when it was written"
    );

    let second = super::add_note(&conn, &key, "Rang, no answer").expect("a second note");
    assert_ne!(second.id, first.id);
    let held = state::read::notes_on(&conn, &key).expect("the notes");
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].body, "Chase this on Friday");

    // The list carries the latest one as its single line under the row.
    assert_eq!(page(&conn, Place::Inbox)[0].note.as_deref(), Some("Rang, no answer"));
}

#[test]
fn deleting_a_note_is_reversible() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "The kitchen", 100);
    let note = super::add_note(&conn, &key, "Chase this on Friday").expect("a note");

    state::write::delete_note(&conn, &note.id).expect("deleted");
    assert!(state::read::notes_on(&conn, &key).expect("the notes").is_empty());

    super::restore_note(&conn, &note).expect("put back");
    let held = state::read::notes_on(&conn, &key).expect("the notes");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id, note.id);
    assert_eq!(held[0].body, "Chase this on Friday");
}

// ---------------------------------------------------------------------------------------------
// Renames
// ---------------------------------------------------------------------------------------------

#[test]
fn a_rename_shows_in_a_list_page_with_the_real_subject_beside_it() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "Re: Fwd: quote v4 FINAL", 100);

    super::set_name(&conn, &key, Some("Kitchen quote")).expect("renamed");

    let row = &page(&conn, Place::Inbox)[0];
    assert_eq!(row.subject, "Kitchen quote");
    assert_eq!(row.original_subject.as_deref(), Some("Re: Fwd: quote v4 FINAL"));

    super::set_name(&conn, &key, None).expect("put back");
    let row = &page(&conn, Place::Inbox)[0];
    assert_eq!(row.subject, "Re: Fwd: quote v4 FINAL");
    assert!(row.original_subject.is_none());
}

// ---------------------------------------------------------------------------------------------
// Merges
// ---------------------------------------------------------------------------------------------

#[test]
fn a_merge_of_three_shows_as_one_thread_named_after_the_longest() {
    let conn = open();
    let first = arrive(&conn, "one@example.org", "Quote", 300);
    let second = arrive(&conn, "two@example.org", "The kitchen quote, revised", 200);
    let third = arrive(&conn, "three@example.org", "Re: quote", 100);

    let keys = vec![first.clone(), second.clone(), third.clone()];
    let (merged_key, sources, rename_before) = super::merge(&conn, &keys, None).expect("a merge");
    assert_eq!(merged_key, first);
    assert_eq!(sources, vec![second.clone(), third.clone()]);
    assert!(rename_before.is_none());

    let inbox = page(&conn, Place::Inbox);
    assert_eq!(inbox.len(), 1, "three threads merged show as one");
    assert_eq!(inbox[0].key, first);
    assert_eq!(inbox[0].subject, "The kitchen quote, revised");
    assert!(inbox[0].merged);
}

#[test]
fn a_chain_of_merges_shows_once() {
    // Two devices can each merge without either meaning to make a chain: c into b, then b into a.
    let conn = open();
    let first = arrive(&conn, "one@example.org", "One", 300);
    let second = arrive(&conn, "two@example.org", "Two", 200);
    let third = arrive(&conn, "three@example.org", "Three", 100);

    super::merge(&conn, &[second.clone(), third.clone()], Some("Two")).expect("c into b");
    super::merge(&conn, &[first.clone(), second.clone()], Some("One")).expect("b into a");

    assert_eq!(keys_in(&conn, Place::Inbox), vec![first]);
}

#[test]
fn an_unmerge_puts_them_all_back() {
    let conn = open();
    let first = arrive(&conn, "one@example.org", "One", 300);
    let second = arrive(&conn, "two@example.org", "Two", 200);
    let third = arrive(&conn, "three@example.org", "Three", 100);
    let keys = vec![first.clone(), second.clone(), third.clone()];
    super::merge(&conn, &keys, None).expect("a merge");

    assert_eq!(
        super::direct_sources(&conn, &first).expect("the sources"),
        vec![third.clone(), second.clone()],
        "the sources are what putting the merge back is made of"
    );
    state::write::unmerge(&conn, &first).expect("unmerged");

    let inbox = keys_in(&conn, Place::Inbox);
    assert_eq!(inbox.len(), 3);
    for key in keys {
        assert!(inbox.contains(&key), "{key} did not come back");
    }
}

#[test]
fn taking_back_one_merge_leaves_the_merge_it_joined() {
    let conn = open();
    let first = arrive(&conn, "one@example.org", "One", 400);
    let second = arrive(&conn, "two@example.org", "Two", 300);
    let third = arrive(&conn, "three@example.org", "Three", 200);
    super::merge(&conn, &[first.clone(), second.clone()], Some("One")).expect("the first merge");

    let (_, sources, rename_before) =
        super::merge(&conn, &[first.clone(), third.clone()], Some("One and three")).expect("more");
    for source in &sources {
        super::unmerge_one(&conn, source).expect("back out");
    }
    super::set_name(&conn, &first, rename_before.as_deref()).expect("the name back");

    let inbox = page(&conn, Place::Inbox);
    assert_eq!(inbox.len(), 2, "the earlier merge is untouched");
    assert_eq!(inbox[0].key, first);
    assert_eq!(inbox[0].subject, "One");
    assert_eq!(inbox[1].key, third);
}

// ---------------------------------------------------------------------------------------------
// Ignore and notify
// ---------------------------------------------------------------------------------------------

#[test]
fn an_ignored_thread_receives_its_messages_and_does_not_return_to_new() {
    let conn = open();
    let ignored = arrive(&conn, "loud@example.org", "The long thread", 100);
    let ordinary = arrive(&conn, "maya@example.org", "Hello", 90);
    state::write::set_thread_flags(&conn, &ignored, Some(true), None).expect("ignored");

    append(&conn, &ignored, 300);
    append(&conn, &ordinary, 200);

    let inbox = page(&conn, Place::Inbox);
    let held = |key: &str| {
        inbox
            .iter()
            .find(|row| row.key == key)
            .expect("the row")
            .clone()
    };

    let loud = held(&ignored);
    assert_eq!(loud.message_count, 2, "the message still arrived and still appended");
    assert!(loud.ignored);
    assert_eq!(loud.group, "seen", "an ignored thread never returns to New for you");
    assert_eq!(held(&ordinary).group, "new");
}

#[test]
fn the_two_switches_are_one_row_and_neither_clears_the_other() {
    let conn = open();
    let key = arrive(&conn, "maya@example.org", "Hello", 100);

    state::write::set_thread_flags(&conn, &key, None, Some(true)).expect("notify");
    state::write::set_thread_flags(&conn, &key, Some(true), None).expect("ignore");

    let flags = state::read::flags_of(&conn, &key).expect("the flags");
    assert!(flags.ignored && flags.notify);

    let row = &page(&conn, Place::Inbox)[0];
    assert!(row.ignored && row.notify);
}
